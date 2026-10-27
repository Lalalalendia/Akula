# Installed Akula Canvas UIA smoke on a disposable GitHub-hosted Windows runner.
# This test uses isolated synthetic .canvas files, with guarded edit/Undo/Redo persistence.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir,
    [switch]$Editable,
    [switch]$IncludeDelete,
    [switch]$IncludeResize,
    [switch]$IncludeConnect
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Installed Canvas acceptance is restricted to hosted Windows CI'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'Canvas UIA test requires Windows PowerShell STA'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([System.IO.Path]::GetFullPath($ExePath) -ne [System.IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed)) {
    throw 'Refusing to test non-installed Akula'
}
if ($IncludeDelete -and !$Editable) { throw 'Canvas Delete acceptance requires Editable mode' }
if ($IncludeResize -and !$Editable) { throw 'Canvas Resize acceptance requires Editable mode' }
if ($IncludeConnect -and !$Editable) { throw 'Canvas Connect acceptance requires Editable mode' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop

. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class AkulaCanvasInstalledPointer {
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
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint count, [In] Input[] inputs, int size);
    [DllImport("user32.dll", SetLastError = true)] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError = true)] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    public static void Click(IntPtr hwnd, int expectedPid, int x, int y) {
        if (IntPtr.Size != 8 || Marshal.SizeOf(typeof(Input)) != 40 || hwnd == IntPtr.Zero)
            throw new InvalidOperationException("Invalid Canvas OS pointer layout/window");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid ||
            GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing Canvas click outside foreground installed Akula PID");
        if (!SetCursorPos(x, y)) throw new InvalidOperationException("Cannot position Canvas OS pointer");
        Thread.Sleep(70);
        IntPtr hit = WindowFromPoint(new Point { X = x, Y = y });
        if (hit == IntPtr.Zero || GetAncestor(hit, 2) != hwnd || GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Canvas click point not inside target HWND");
        var sequence = new Input[] { new Input { Type=0, Flags=0x0002 }, new Input { Type=0, Flags=0x0004 } };
        if (SendInput(2, sequence, Marshal.SizeOf(typeof(Input))) != 2)
            throw new InvalidOperationException("Incomplete physical Canvas pointer input");
    }
    public static void Drag(IntPtr hwnd, int expectedPid, int x0, int y0, int x1, int y1) {
        if (IntPtr.Size != 8 || Marshal.SizeOf(typeof(Input)) != 40 || hwnd == IntPtr.Zero)
            throw new InvalidOperationException("Invalid Canvas OS drag layout/window");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid ||
            GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing Canvas drag outside foreground installed Akula PID");
        foreach (var point in new[] { new Point { X=x0, Y=y0 }, new Point { X=x1, Y=y1 } }) {
            IntPtr hit = WindowFromPoint(point);
            if (hit == IntPtr.Zero || GetAncestor(hit, 2) != hwnd)
                throw new InvalidOperationException("Canvas drag endpoint falls outside target HWND");
        }
        if (!SetCursorPos(x0, y0)) throw new InvalidOperationException("Canvas pointer could not reach resize grip");
        Thread.Sleep(85);
        bool pressed = false;
        try {
            if (SendInput(1, new Input[] { new Input { Type=0, Flags=0x0002 } }, Marshal.SizeOf(typeof(Input))) != 1)
                throw new InvalidOperationException("Canvas OS resize mouse down failed");
            pressed = true;
            for (int i = 1; i <= 10; i++) {
                int x = x0 + (int)Math.Round((x1-x0) * (double)i / 10);
                int y = y0 + (int)Math.Round((y1-y0) * (double)i / 10);
                if (!SetCursorPos(x, y)) throw new InvalidOperationException("Canvas OS resize mouse move failed");
                Thread.Sleep(40);
            }
        } finally {
            if (pressed &&
                SendInput(1, new Input[] { new Input { Type=0, Flags=0x0004 } }, Marshal.SizeOf(typeof(Input))) != 1)
                throw new InvalidOperationException("Canvas OS resize mouse up failed");
        }
    }
}
'@

function Wait-CanvasNodeCount {
    param([string]$Path, [int]$Count, [System.Diagnostics.Process]$Process)
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        Start-Sleep -Milliseconds 150
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Akula exited during installed Canvas editing' }
        try {
            $value = [System.IO.File]::ReadAllText($Path) | ConvertFrom-Json
            if (@($value.nodes).Count -eq $Count) { return $value }
        } catch [System.IO.IOException] { }
    }
    throw "Installed Canvas did not persist the expected $Count nodes"
}

function Invoke-CanvasToolbar {
    param([int]$Index, [System.Diagnostics.Process]$Process,
        $CanvasBounds, $WindowBounds)
    $hwnd = [IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    $active = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { $active = $true; break }
    }
    if (!$active) { throw 'Akula did not become foreground for synthetic Canvas click' }
    $dpi = [AkulaCanvasInstalledPointer]::GetDpiForWindow($hwnd)
    if ($dpi -lt 96 -or $dpi -gt 480) { throw "Unexpected Canvas window DPI $dpi" }
    $scale = [double]$dpi / 96.0
    # Indices 0..2: original toolbar. 4: lower-row Delete. 5:
    # verified synthetic first-card center using CanvasPage::fit.
    if ($Index -notin @(0, 1, 2, 4, 5, 6, 7)) { throw 'Non-whitelisted Canvas click target' }
    $w = $CanvasBounds.Width / $scale
    $h = $CanvasBounds.Height / $scale
    $localX = [Math]::Max(0.0, $w - 317.0) + 36.0 + $Index * 77.0
    $localY = 25.0
    if ($Index -eq 4) {
        $localX = [Math]::Max(0.0, $w - 86.0) + 36.0
        $localY = 62.0
    } elseif ($Index -eq 5) {
        # Fixture bounding group = 0..420 x 0..260; text card 'first'
        # center = (114, 82). Transform with fitted Canvas camera.
        $zoom = [Math]::Min([Math]::Max(1.0, $w - 96.0) / 420.0,
                            [Math]::Max(1.0, $h - 96.0) / 260.0)
        $zoom = [Math]::Min(8.0, [Math]::Max(0.05, $zoom))
        $localX = $w / 2.0 - 96.0 * $zoom
        $localY = $h / 2.0 - 48.0 * $zoom
    } elseif ($Index -eq 6) {
        # Second toolbar row: Connect button to the left of Delete.
        $localX = [Math]::Max(0.0, $w - 163.0) + 36.0
        $localY = 62.0
    } elseif ($Index -eq 7) {
        # The synthetic new text card is created at fitted board center
        # (210,130) with size 240x120. Its center is (330,190).
        $zoom = [Math]::Min([Math]::Max(1.0, $w - 96.0) / 420.0,
                            [Math]::Max(1.0, $h - 96.0) / 260.0)
        $zoom = [Math]::Min(8.0, [Math]::Max(0.05, $zoom))
        $localX = $w / 2.0 + 120.0 * $zoom
        $localY = $h / 2.0 + 60.0 * $zoom
    }
    $x = [int][Math]::Round($CanvasBounds.Left + $localX * $scale)
    $y = [int][Math]::Round($CanvasBounds.Top + $localY * $scale)
    if ($x -lt $CanvasBounds.Left + 5 -or $x -ge $CanvasBounds.Right - 5 -or
        $y -lt $CanvasBounds.Top + 5 -or $y -ge $CanvasBounds.Bottom - 5 -or
        $x -lt $WindowBounds.Left + 5 -or $x -ge $WindowBounds.Right - 5 -or
        $y -lt $WindowBounds.Top + 5 -or $y -ge $WindowBounds.Bottom - 5) {
        throw "Canvas toolbar $Index outside visible UIA HWND: x=$x y=$y dpi=$dpi"
    }
    [AkulaCanvasInstalledPointer]::Click($hwnd, $Process.Id, $x, $y)
}


function Invoke-CanvasResize {
    param([System.Diagnostics.Process]$Process, $CanvasBounds, $WindowBounds)
    $hwnd = [IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    $foreground = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 120
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { $foreground = $true; break }
    }
    if (!$foreground) { throw 'Canvas window is not foreground for OS resize' }
    $dpi = [AkulaCanvasInstalledPointer]::GetDpiForWindow($hwnd)
    if ($dpi -lt 96 -or $dpi -gt 480) { throw "Unexpected Canvas Resize DPI $dpi" }
    $scale = [double]$dpi / 96.0
    $w = $CanvasBounds.Width / $scale
    $h = $CanvasBounds.Height / $scale
    # The original synthetic board bounds are 0..420 by 0..260.
    # Text node "first" occupies 24..204 by 32..132. Its selected grip is
    # the inner 12x12 screen-pixel corner; the fitted camera stays fixed
    # across the preceding Add/Undo/Redo actions.
    $zoom = [Math]::Min([Math]::Max(1.0, $w - 96.0) / 420.0,
                        [Math]::Max(1.0, $h - 96.0) / 260.0)
    $zoom = [Math]::Min(8.0, [Math]::Max(0.05, $zoom))
    $x0 = [int][Math]::Round($CanvasBounds.Left + ($w / 2.0 - 6.0 * $zoom - 6.0) * $scale)
    $y0 = [int][Math]::Round($CanvasBounds.Top + ($h / 2.0 + 2.0 * $zoom - 6.0) * $scale)
    $x1 = $x0 + [int][Math]::Round(65.0 * $scale)
    $y1 = $y0 + [int][Math]::Round(45.0 * $scale)
    foreach ($p in @(@($x0,$y0),@($x1,$y1))) {
        if ($p[0] -lt $CanvasBounds.Left + 5 -or $p[0] -ge $CanvasBounds.Right - 5 -or
            $p[1] -lt $CanvasBounds.Top + 5 -or $p[1] -ge $CanvasBounds.Bottom - 5 -or
            $p[0] -lt $WindowBounds.Left + 5 -or $p[0] -ge $WindowBounds.Right - 5 -or
            $p[1] -lt $WindowBounds.Top + 5 -or $p[1] -ge $WindowBounds.Bottom - 5) {
            throw "Canvas Resize drag endpoint outside visible UIA widget: $($p[0]),$($p[1])"
        }
    }
    [AkulaCanvasInstalledPointer]::Drag($hwnd, $Process.Id, $x0, $y0, $x1, $y1)
}

$populatedPath = Join-Path $VaultPath 'canvas-installed-populated.canvas'
$emptyPath = Join-Path $VaultPath 'canvas-installed-empty.canvas'
foreach ($path in @($populatedPath, $emptyPath)) {
    if (Test-Path -LiteralPath $path) { throw "Canvas fixture is not clean: $path" }
}
$populated = @'
{
  "nodes": [
    {"id":"group","type":"group","x":0,"y":0,"width":420,"height":260,"label":"Project"},
    {"id":"first","type":"text","x":24,"y":32,"width":180,"height":100,"text":"First synthetic card"},
    {"id":"second","type":"text","x":230,"y":130,"width":170,"height":90,"text":"Second synthetic card"}
  ],
  "edges": [{"id":"edge","fromNode":"first","toNode":"second","fromSide":"right","toSide":"left","toEnd":"arrow"}],
  "vendor":{"opaque":"preserve"}
}
'@
$utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($populatedPath, $populated, $utf8)
[System.IO.File]::WriteAllText($emptyPath, '{"nodes":[],"edges":[]}', $utf8)
$checks = @(
    @{ Path = $populatedPath; Label = 'Canvas: 3 nodes, 1 edges'; Opposite = 'Canvas: 0 nodes, 0 edges' },
    @{ Path = $emptyPath; Label = 'Canvas: 0 nodes, 0 edges'; Opposite = 'Canvas: 3 nodes, 1 edges' }
)

foreach ($test in $checks) {
    $originalHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
    $expectedHash = $originalHash
    $proc = $null
    try {
        $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $test.Path
        $proc = Start-Process -FilePath $installed -ArgumentList $arguments -PassThru
        $root = $null
        for ($i = 0; $i -lt 100; $i++) {
            Start-Sleep -Milliseconds 250
            $proc.Refresh()
            if ($proc.HasExited) { throw "Installed Akula exited during Canvas open: $($proc.ExitCode)" }
            if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and $proc.MainWindowTitle -like '*Akula*') {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
                if ($null -ne $root) { break }
            }
        }
        if ($null -eq $root) { throw 'Installed Akula did not expose a Canvas UIA window' }
        $condition = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty,
            [string]$test.Label
        )
        $found = $null
        $visible = $false
        $diagnostic = 'element_not_found'
        for ($i = 0; $i -lt 100; $i++) {
            Start-Sleep -Milliseconds 200
            $proc.Refresh()
            if ($proc.HasExited) { throw 'Akula exited before Canvas accessibility appeared' }
            try {
                $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                if ($null -eq $found) { continue }
                $bounds = $found.Current.BoundingRectangle
                $windowBounds = $root.Current.BoundingRectangle
                # UIA may report a partly clipped child rectangle. Require a
                # substantial, genuinely visible intersection rather than
                # demanding that the complete child fit inside the HWND.
                $width = [Math]::Max(0.0, [Math]::Min($bounds.Right, $windowBounds.Right) -
                    [Math]::Max($bounds.Left, $windowBounds.Left))
                $height = [Math]::Max(0.0, [Math]::Min($bounds.Bottom, $windowBounds.Bottom) -
                    [Math]::Max($bounds.Top, $windowBounds.Top))
                $offscreen = [bool]$found.Current.IsOffscreen
                $diagnostic = ('offscreen={0};canvas=({1},{2},{3},{4});window=({5},{6},{7},{8});intersection=({9},{10})' -f
                    $offscreen, [int]$bounds.Left, [int]$bounds.Top, [int]$bounds.Width, [int]$bounds.Height,
                    [int]$windowBounds.Left, [int]$windowBounds.Top, [int]$windowBounds.Width, [int]$windowBounds.Height,
                    [int]$width, [int]$height)
                if (!$offscreen -and !$bounds.IsEmpty -and !$windowBounds.IsEmpty -and
                    $width -ge 100 -and $height -ge 100) {
                    $visible = $true
                    break
                }
            } catch [System.Windows.Automation.ElementNotAvailableException] {
                $found = $null
                $diagnostic = 'element_temporarily_unavailable'
            }
        }
        if (!$visible) {
            throw "Installed Canvas is not visibly laid out inside its window: $diagnostic"
        }
        $other = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty, [string]$test.Opposite
        )
        if ($null -ne $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $other)) {
            throw 'Canvas accessibility presented stale content from another board'
        }
        if ((Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash -ne $originalHash) {
            throw 'Opening installed Canvas modified the source document'
        }
        Write-Host ("AKULA_WINDOWS_CANVAS_VIEW_PASS file={0} visible=True source_unchanged=True" -f
            [System.IO.Path]::GetFileName($test.Path))

        if ($Editable -and $test.Path -eq $populatedPath) {
            # Real installed OS mouse events, not direct calls to Rust session
            # or JSON-file editing. The application owns File Gate writes.
            Invoke-CanvasToolbar -Index 0 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
            $added = Wait-CanvasNodeCount -Path $test.Path -Count 4 -Process $proc
            if ($added.vendor.opaque -ne 'preserve' -or
                $added.nodes[0].id -ne 'group' -or $added.nodes[1].id -ne 'first' -or
                $added.nodes[2].id -ne 'second' -or $added.edges[0].id -ne 'edge' -or
                $added.nodes[3].type -ne 'text' -or $added.nodes[3].text -ne 'New card' -or
                [string]::IsNullOrWhiteSpace([string]$added.nodes[3].id)) {
                throw 'Installed Canvas Add Text lost original nodes, connections or unknown fields'
            }
            $newNodeId = [string]$added.nodes[3].id
            Invoke-CanvasToolbar -Index 1 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
            $undone = Wait-CanvasNodeCount -Path $test.Path -Count 3 -Process $proc
            if ($undone.vendor.opaque -ne 'preserve' -or $undone.edges[0].id -ne 'edge') {
                throw 'Installed Canvas Undo lost source extension or edge'
            }
            Invoke-CanvasToolbar -Index 2 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
            $redone = Wait-CanvasNodeCount -Path $test.Path -Count 4 -Process $proc
            if ($redone.nodes[3].id -ne $newNodeId -or $redone.vendor.opaque -ne 'preserve') {
                throw 'Installed Canvas Redo restored the wrong card or lost extension data'
            }
            $expectedHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
            if ($expectedHash -eq $originalHash) { throw 'Installed Canvas mouse edits did not change source bytes' }
            Write-Host 'AKULA_WINDOWS_CANVAS_EDIT_PASS add=1 undo=1 redo=1 file_gate=1 vendor_preserved=1'
            if ($IncludeResize) {
                # Click real text card first, then drag only its selected
                # bottom-right grip. No direct mutation of fixture JSON.
                Invoke-CanvasToolbar -Index 5 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                Invoke-CanvasResize -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $resized = $null
                for ($attempt = 0; $attempt -lt 80; $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $proc.Refresh()
                    if ($proc.HasExited) { throw 'Installed Akula exited during Canvas resize' }
                    try {
                        $candidate = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                        $first = @($candidate.nodes | Where-Object { $_.id -eq 'first' })
                        if ($first.Count -eq 1 -and $first[0].width -gt 180 -and $first[0].height -gt 100) {
                            $resized = $candidate
                            break
                        }
                    } catch [System.IO.IOException] { }
                }
                if ($null -eq $resized -or $resized.vendor.opaque -ne 'preserve' -or
                    $resized.edges[0].id -ne 'edge') {
                    throw 'Installed Canvas pointer Resize failed to save bounded size without data loss'
                }
                $width = [int](@($resized.nodes | Where-Object { $_.id -eq 'first' })[0].width)
                $height = [int](@($resized.nodes | Where-Object { $_.id -eq 'first' })[0].height)
                Invoke-CanvasToolbar -Index 1 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $undoneSize = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                for ($attempt = 0; $attempt -lt 60 -and
                    (@($undoneSize.nodes | Where-Object { $_.id -eq 'first' })[0].width -ne 180); $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $undoneSize = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                }
                if (@($undoneSize.nodes | Where-Object { $_.id -eq 'first' })[0].height -ne 100 -or
                    @($undoneSize.nodes | Where-Object { $_.id -eq 'first' })[0].width -ne 180) {
                    throw 'Canvas resize Undo did not restore both dimensions'
                }
                Invoke-CanvasToolbar -Index 2 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $redoneSize = $null
                for ($attempt = 0; $attempt -lt 60; $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $candidate = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                    $first = @($candidate.nodes | Where-Object { $_.id -eq 'first' })
                    if ($first.Count -eq 1 -and $first[0].width -eq $width -and $first[0].height -eq $height) {
                        $redoneSize = $candidate
                        break
                    }
                }
                if ($null -eq $redoneSize -or $redoneSize.vendor.opaque -ne 'preserve') {
                    throw 'Canvas resize Redo did not restore saved width/height and vendor data'
                }
                $expectedHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
                Write-Host 'AKULA_WINDOWS_CANVAS_RESIZE_PASS select=1 pointer_drag=1 undo=1 redo=1 file_gate=1'
            }
            if ($IncludeConnect) {
                # Connect an existing text card to the newly added text card.
                # Arm one-shot mode, click target, and verify actual disk JSON.
                Invoke-CanvasToolbar -Index 5 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                Invoke-CanvasToolbar -Index 6 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                Invoke-CanvasToolbar -Index 7 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $linked = $null
                for ($attempt = 0; $attempt -lt 80; $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $proc.Refresh()
                    if ($proc.HasExited) { throw 'Akula exited before installed Canvas Connect persisted' }
                    try {
                        $candidate = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                        if (@($candidate.edges).Count -eq 2) { $linked = $candidate; break }
                    } catch [System.IO.IOException] { }
                }
                if ($null -eq $linked -or $linked.vendor.opaque -ne 'preserve' -or
                    $linked.edges[0].id -ne 'edge' -or
                    $linked.edges[1].fromNode -ne 'first' -or
                    $linked.edges[1].toNode -ne $newNodeId -or
                    $linked.edges[1].toEnd -ne 'arrow') {
                    throw 'Installed Canvas Connect failed to create standard edge while preserving vendor data'
                }
                $edgeId = [string]$linked.edges[1].id
                if ([string]::IsNullOrWhiteSpace($edgeId)) { throw 'Installed Canvas Connect edge has no ID' }
                Invoke-CanvasToolbar -Index 1 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $edgeUndo = $null
                for ($attempt = 0; $attempt -lt 60; $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $candidate = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                    if (@($candidate.edges).Count -eq 1) { $edgeUndo = $candidate; break }
                }
                if ($null -eq $edgeUndo -or $edgeUndo.edges[0].id -ne 'edge') {
                    throw 'Installed Canvas Connect Undo failed to restore original edges'
                }
                Invoke-CanvasToolbar -Index 2 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $edgeRedo = $null
                for ($attempt = 0; $attempt -lt 60; $attempt++) {
                    Start-Sleep -Milliseconds 150
                    $candidate = [System.IO.File]::ReadAllText($test.Path) | ConvertFrom-Json
                    if (@($candidate.edges).Count -eq 2) { $edgeRedo = $candidate; break }
                }
                if ($null -eq $edgeRedo -or $edgeRedo.edges[1].id -ne $edgeId -or
                    $edgeRedo.vendor.opaque -ne 'preserve') {
                    throw 'Installed Canvas Connect Redo lost generated edge ID or vendor property'
                }
                $expectedHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
                Write-Host 'AKULA_WINDOWS_CANVAS_CONNECT_PASS source=1 target=1 edge=1 undo=1 redo=1 file_gate=1'
            }
            if ($IncludeDelete) {
                Invoke-CanvasToolbar -Index 5 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                # Selection is in-memory: it must not alter the 4-card file.
                if (@((Get-Content -LiteralPath $test.Path -Raw | ConvertFrom-Json).nodes).Count -ne 4) {
                    throw 'Canvas card selection unexpectedly modified the file'
                }
                Invoke-CanvasToolbar -Index 4 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $removed = Wait-CanvasNodeCount -Path $test.Path -Count 3 -Process $proc
                if (@($removed.edges).Count -ne 0 -or $removed.vendor.opaque -ne 'preserve' -or
                    @($removed.nodes | Where-Object { $_.id -eq 'first' }).Count -ne 0 -or
                    @($removed.nodes | Where-Object { $_.id -eq $newNodeId }).Count -ne 1) {
                    throw 'Installed Canvas Delete lost an unrelated card, failed to remove the edge, or damaged vendor JSON'
                }
                Invoke-CanvasToolbar -Index 1 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $restored = Wait-CanvasNodeCount -Path $test.Path -Count 4 -Process $proc
                if ($restored.edges[0].id -ne 'edge' -or $restored.nodes[1].id -ne 'first') {
                    throw 'Installed Canvas Delete Undo failed to restore original node and connection'
                }
                Invoke-CanvasToolbar -Index 2 -Process $proc -CanvasBounds $bounds -WindowBounds $windowBounds
                $removedAgain = Wait-CanvasNodeCount -Path $test.Path -Count 3 -Process $proc
                if (@($removedAgain.edges).Count -ne 0 -or
                    @($removedAgain.nodes | Where-Object { $_.id -eq 'first' }).Count -ne 0) {
                    throw 'Installed Canvas Delete Redo did not remove the same node and edge'
                }
                $expectedHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
                Write-Host 'AKULA_WINDOWS_CANVAS_DELETE_PASS selected=1 delete=1 undo=1 redo=1 linked_edge=1 file_gate=1'
            }

        }

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
    if ((Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash -ne $expectedHash) {
        throw 'Closing installed Canvas changed the expected saved revision'
    }
}
Write-Host ("AKULA_WINDOWS_CANVAS_INSTALLED_PASS populated=1 empty=1 uia=1 file_gate_edit={0} source_sha256=1" -f $Editable.IsPresent)
