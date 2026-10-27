# DESK-15: two real installed GUI tabs, separate File Gate ACK and crash recovery.
# Synthetic data on a disposable GitHub-hosted x64 Windows runner only.
param([Parameter(Mandatory=$true)][string]$ExePath)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    [IntPtr]::Size -ne 8 -or $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Two-tab DESK-15 requires hosted x64 Windows PowerShell STA'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed -PathType Leaf)) {
    throw 'Two-tab recovery requires the installed Akula executable'
}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

# INPUT: 40 bytes on Win64. The keyboard/mouse fields overlap in a native union.
# Send only the fixed Ctrl+Shift+left-click used to open a known disposable note
# in a new tab. Never build a general-purpose desktop injection helper.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaDesk15NewTabClick {
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input {
        [FieldOffset(0)] public uint Kind;
        [FieldOffset(8)] public ushort Key;
        [FieldOffset(12)] public uint KeyFlags;
        [FieldOffset(20)] public uint MouseFlags;
    }
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", SetLastError=true)]
    private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint SendInput(uint count, [In] Input[] inputs, int size);
    public static void Click(IntPtr hwnd, int expectedPid, int x, int y) {
        if (hwnd == IntPtr.Zero || Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Invalid x64 pointer input target");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid
            || GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Two-tab OS click target is not foreground Akula");
        if (!SetCursorPos(x, y))
            throw new InvalidOperationException("Cannot position pointer on verified tree row");
        Input[] events = new Input[] {
            new Input { Kind = 1, Key = 0x11 }, // Ctrl
            new Input { Kind = 1, Key = 0x10 }, // Shift
            new Input { Kind = 0, MouseFlags = 0x0002 }, // left down
            new Input { Kind = 0, MouseFlags = 0x0004 }, // left up
            new Input { Kind = 1, Key = 0x10, KeyFlags = 0x0002 },
            new Input { Kind = 1, Key = 0x11, KeyFlags = 0x0002 }
        };
        uint sent = SendInput((uint)events.Length, events, Marshal.SizeOf(typeof(Input)));
        if (sent != (uint)events.Length)
            throw new InvalidOperationException("Two-tab OS click inserted " + sent +
                "/" + events.Length + "; Win32 error " + Marshal.GetLastWin32Error());
    }
}
'@ -ErrorAction Stop

$root = Join-Path $env:RUNNER_TEMP ('akula-desk15-tabs-' + [Guid]::NewGuid().ToString('N'))
$vault = Join-Path $root 'vault'
$data = Join-Path $root 'data'
[IO.Directory]::CreateDirectory($vault) | Out-Null
[IO.Directory]::CreateDirectory($data) | Out-Null
$first = Join-Path $vault 'First note.md'
$second = Join-Path $vault 'Second note.md'
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText($first, 'original first tab', $utf8)
[IO.File]::WriteAllText($second, 'original second tab', $utf8)
$firstHash = (Get-FileHash -LiteralPath $first -Algorithm SHA256).Hash
$secondHash = (Get-FileHash -LiteralPath $second -Algorithm SHA256).Hash
$settings = Join-Path $data 'settings.json'
[IO.File]::WriteAllText($settings, '{"editor":{"saveDebounceMs":120000}}', $utf8)
$shadowBase = Join-Path $data 'akula-note-recovery-v1'
$markerA = 'DESK15_TAB_A_' + [Guid]::NewGuid().ToString('N')
$markerB = 'DESK15_TAB_B_' + [Guid]::NewGuid().ToString('N')
$app = $null

function Start-Installed([string]$note,[string]$title) {
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $proc = Start-Process -FilePath $installed -ArgumentList $args -PassThru
    for ($i=0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Installed Akula exited before the requested test note' }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like ($title + '*') -and
            $proc.MainWindowTitle -like '*Akula*') { return $proc }
    }
    throw 'Installed Akula did not show the requested two-tab note'
}
function Root-Ui([Diagnostics.Process]$Process) {
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw 'Installed Akula has no expected HWND'
    }
    $tree = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Process.MainWindowHandle)
    if ($null -eq $tree) { throw 'Installed Akula has no UIA tree' }
    return $tree
}
function Find-Named([Diagnostics.Process]$Process,[Windows.Automation.ControlType]$Kind,[string]$Name) {
    $tree = Root-Ui $Process
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty, $Kind)
    $items = $tree.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
    $matches = @()
    for ($i=0; $i -lt $items.Count; $i++) {
        $item = $items.Item($i)
        if ([string]$item.Current.Name -ceq $Name -and
            $item.Current.IsEnabled -and !$item.Current.IsOffscreen) { $matches += $item }
    }
    if ($matches.Count -gt 1) { throw 'Ambiguous target in installed Akula UIA tree' }
    if ($matches.Count -eq 1) { return $matches[0] }
    return $null
}
function Activate-Other-Tab([Diagnostics.Process]$Process) {
    $target = $null
    for ($i=0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        $target = Find-Named $Process ([Windows.Automation.ControlType]::TreeItem) 'Second note'
        if ($null -ne $target) { break }
    }
    if ($null -eq $target) { throw 'Second note not accessible in real file tree' }
    $bounds = $target.Current.BoundingRectangle
    $window = (Root-Ui $Process).Current.BoundingRectangle
    if ($bounds.IsEmpty -or $bounds.Width -lt 20 -or $bounds.Height -lt 10 -or
        $bounds.Left -lt $window.Left -or $bounds.Right -gt $window.Right -or
        $bounds.Top -lt $window.Top -or $bounds.Bottom -gt $window.Bottom) {
        throw 'Second note tree item is not a contained visible OS pointer target'
    }
    $hwnd=[IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd,9) | Out-Null
    $foreground=$false
    for ($i=0; $i -lt 15; $i++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 200
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { $foreground=$true; break }
    }
    if (!$foreground) { throw 'Cannot focus installed Akula for two-tab pointer event' }
    $x=[int][Math]::Round($bounds.Left + [Math]::Min(50.0, $bounds.Width / 2.0))
    $y=[int][Math]::Round($bounds.Top + $bounds.Height / 2.0)
    [AkulaDesk15NewTabClick]::Click($hwnd,$Process.Id,$x,$y)
    for ($i=0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Akula crashed after new-tab OS click' }
        if ($Process.MainWindowTitle -like 'Second note*') { break }
    }
    if ($Process.MainWindowTitle -notlike 'Second note*') { throw 'OS Ctrl+Shift+click did not open second note' }
    $tree = Root-Ui $Process
    $cond = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::TabItem)
    $tabs = $tree.FindAll([Windows.Automation.TreeScope]::Descendants,$cond)
    $names = @()
    for ($i=0; $i -lt $tabs.Count; $i++) { $names += [string]$tabs.Item($i).Current.Name }
    if ($names.Count -lt 2 -or $names -notcontains 'First note' -or $names -notcontains 'Second note') {
        throw 'Two real document tabs were not present after OS Ctrl+Shift+click'
    }
}
function Focus-Editor([Diagnostics.Process]$Process) {
    $tree = Root-Ui $Process
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)
    $items = $tree.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
    $best = $null
    $bestArea = 0.0
    for ($i=0; $i -lt $items.Count; $i++) {
        $item = $items.Item($i)
        $bounds = $item.Current.BoundingRectangle
        $area = $bounds.Width * $bounds.Height
        if ($item.Current.IsKeyboardFocusable -and !$item.Current.IsOffscreen -and
            $bounds.Width -gt 200 -and $bounds.Height -gt 100 -and $area -gt $bestArea) {
            $best = $item
            $bestArea = $area
        }
    }
    if ($null -eq $best) { throw 'Installed note editor is not a focusable multiline UIA text input' }
    $best.SetFocus()
}
function Wait-Shadow([string]$marker,[Diagnostics.Process]$Process) {
    for ($i=0; $i -lt 120; $i++) {
        Start-Sleep -Milliseconds 200
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Akula exited before durable shadow was staged' }
        if (!(Test-Path -LiteralPath $shadowBase)) { continue }
        $records = @(Get-ChildItem -LiteralPath $shadowBase -Filter '*.shadow' -File -Recurse -ErrorAction Stop)
        foreach ($record in $records) {
            $bytes = [IO.File]::ReadAllBytes($record.FullName)
            $wire = [Text.Encoding]::UTF8.GetString($bytes)
            if ($wire.StartsWith('AKULA-NOTE-SHADOW-V1') -and $wire.Contains($marker)) {
                return $record.FullName
            }
        }
    }
    throw 'The complete dirty note never appeared in a durable shadow'
}
function Find-Restore([Diagnostics.Process]$Process) {
    return Find-Named $Process ([Windows.Automation.ControlType]::Button) 'Restore draft'
}

try {
    $app=Start-Installed $first 'First note'
    Send-AkulaBetaText -Process $app -Text $markerA
    $null=Wait-Shadow $markerA $app
    if ((Get-FileHash -LiteralPath $first -Algorithm SHA256).Hash -ne $firstHash) {
        throw 'First tab unexpectedly saved before the OS tab switch'
    }
    Activate-Other-Tab $app
    $firstSaved=$false
    for ($i=0; $i -lt 140; $i++) {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Installed Akula exited during detached File Gate save' }
        if (!(Test-Path -LiteralPath $shadowBase)) { continue }
        $left = @(Get-ChildItem -LiteralPath $shadowBase -Filter '*.shadow' -File -Recurse)
        $disk=[IO.File]::ReadAllText($first)
        if ($disk.Contains($markerA) -and $disk.Contains('original first tab') -and $left.Count -eq 0) {
            $firstSaved=$true
            break
        }
    }
    if (!$firstSaved) { throw 'First tab was saved but its detached shadow ACK cleanup was not proven' }
    if ((Get-FileHash -LiteralPath $second -Algorithm SHA256).Hash -ne $secondHash) {
        throw 'Second tab was modified before editor focus'
    }
    Focus-Editor $app
    Send-AkulaBetaText -Process $app -Text $markerB
    $null=Wait-Shadow $markerB $app
    if ((Get-FileHash -LiteralPath $second -Algorithm SHA256).Hash -ne $secondHash) {
        throw 'Second unsaved tab was written to disk before the crash'
    }
    $app.Kill()
    if (!$app.WaitForExit(20000)) { throw 'Hard-killed Akula did not exit' }
    $app.Dispose(); $app=$null
    if ((Get-FileHash -LiteralPath $second -Algorithm SHA256).Hash -ne $secondHash -or
        ![IO.File]::ReadAllText($first).Contains($markerA)) {
        throw 'Crash changed one or both files before user approval'
    }

    [IO.File]::WriteAllText($settings,'{"editor":{"saveDebounceMs":250}}',$utf8)
    $app=Start-Installed $second 'Second note'
    $restore=$null
    for ($i=0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        if ((Get-FileHash -LiteralPath $second -Algorithm SHA256).Hash -ne $secondHash) {
            throw 'Reopening applied second-tab recovery without confirmation'
        }
        $restore=Find-Restore $app
        if ($null -ne $restore) { break }
    }
    if ($null -eq $restore) { throw 'Second tab has no real recovery UIA confirmation after crash' }
    $invoke=$restore.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $invoke) { throw 'Second-tab recovery button lacks InvokePattern' }
    ([Windows.Automation.InvokePattern]$invoke).Invoke()
    $retired=$false
    for ($i=0; $i -lt 140; $i++) {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Akula exited while restoring second tab' }
        if (![IO.File]::ReadAllText($second).Contains($markerB)) { continue }
        $left=@(Get-ChildItem -LiteralPath $shadowBase -Filter '*.shadow' -File -Recurse)
        if ($left.Count -eq 0) { $retired=$true; break }
    }
    if (!$retired -or ![IO.File]::ReadAllText($second).Contains('original second tab')) {
        throw 'Second-tab restored bytes or verified shadow retirement failed'
    }
    if (![IO.File]::ReadAllText($first).Contains($markerA)) {
        throw 'Second-tab recovery changed previously saved first tab'
    }
    # File Gate bytes alone do not prove that the *installed editor* displays
    # the recovered text. Read the actual foreground widget with native OS keys.
    Focus-Editor $app
    $app.Refresh()
    $hwnd = [IntPtr]$app.MainWindowHandle
    if ($app.MainWindowTitle -notlike 'Second note*' -or
        [AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
        throw 'Cannot verify foreground second-tab editor for recovered GUI readback'
    }
    $poison = 'DESK15_TAB_CLIPBOARD_POISON_' + [Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $poison -ErrorAction Stop
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $app.Id, [uint16]0x41) # Ctrl+A
    Start-Sleep -Milliseconds 350
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $app.Id, [uint16]0x43) # Ctrl+C
    $readback = $null
    for ($i=0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 200
        try {
            $clipboard = Get-Clipboard -Raw -ErrorAction Stop
            if (![string]::IsNullOrEmpty($clipboard) -and $clipboard -cne $poison -and
                $clipboard.Contains($markerB)) { $readback = $clipboard; break }
        } catch [Runtime.InteropServices.ExternalException] { }
    }
    if ($null -eq $readback) { throw 'Second-tab GUI never copied the recovered draft' }
    $normal = {
        param([string]$value)
        ([regex]::Replace($value, '\r\n?', [string][char]10)).TrimEnd([char]10)
    }
    if (![string]::Equals((& $normal $readback),
        (& $normal ([IO.File]::ReadAllText($second))), [StringComparison]::Ordinal)) {
        throw 'Second-tab installed editor readback differs from exact recovered File Gate bytes'
    }
    Write-Host 'AKULA_WINDOWS_DESK15_TWO_TABS_PASS os_ctrlshift_click=1 tabs=2 detached_ack=1 crash=1 second_restore=1 file_gate=1 shadows_retired=1 gui_exact=1'
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000)) {
        throw 'Installed Akula refused graceful close after two-tab test'
    }
    $app.Dispose(); $app=$null
} finally {
    if ($null -ne $app) {
        $app.Refresh()
        if (!$app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) }
        $app.Dispose()
    }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
