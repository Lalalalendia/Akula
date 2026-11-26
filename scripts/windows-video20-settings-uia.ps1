# VIDEO-20: real installed-feature Akula Settings -> opt-in -> remove.
# Non-release, disposable hosted Windows GUI test. No YouTube requests.
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$VaultPath,
    [Parameter(Mandatory=$true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO20 UIA requires exact-head hosted Windows pull request'
}
if ($PSVersionTable.PSEdition -cne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA -or
    [IntPtr]::Size -ne 8) {
    throw 'VIDEO20 UIA requires x64 Windows PowerShell 5.1 STA'
}
$expectedExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ine [IO.Path]::GetFullPath($expectedExe) -or
    !(Test-Path -LiteralPath $expectedExe -PathType Leaf) -or
    (Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue)) {
    throw 'VIDEO20 UIA refuses unexpected executable or running Akula'
}
$tempRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
foreach ($path in @($VaultPath, $DataDir)) {
    if (![IO.Path]::GetFullPath($path).StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $path)) {
        throw 'VIDEO20 UIA only accepts fresh RUNNER_TEMP profiles'
    }
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

# Native pointer click: AccessKit buttons may not support UIA SetFocus.
# The click is permitted only inside the verified foreground Akula HWND.
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class AkulaVideo20Mouse {
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(20)] public uint Flags;
        [FieldOffset(16)] public uint Data;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }
    [DllImport("user32.dll", SetLastError=true)] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint owner);
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, [In] Input[] data, int size);
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    delegate bool EnumChildProc(IntPtr child, IntPtr param);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr child);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr param);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    static IntPtr FindControl(IntPtr dialog, int id) {
        var direct = GetDlgItem(dialog, id);
        if (direct != IntPtr.Zero) return direct;
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(dialog, (hwnd, arg) => {
            if (GetDlgCtrlID(hwnd) == id) { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void Scroll(IntPtr hwnd, int pid, int x, int y, bool up) {
        uint owner;
        if (GetForegroundWindow() != hwnd || GetWindowThreadProcessId(hwnd, out owner) == 0 ||
            owner != (uint)pid || !SetCursorPos(x, y))
            throw new InvalidOperationException("VIDEO21 refuses unowned scroll");
        Thread.Sleep(75);
        var wheel = new Input[] {new Input {Type=0,Flags=0x0800,Data=unchecked((uint)(up ? 120 : -120))}};
        if (SendInput(1, wheel, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("VIDEO21 physical scroll failed");
    }
    public static void ClickConsent(IntPtr dialog, int pid, bool yes) {
        // rfd 0.16 uses Win32 IDYES=6 and IDNO=7 with both
        // MessageBoxW and TaskDialogIndirect on Windows.
        IntPtr affirmative = FindControl(dialog, 6);
        IntPtr negative = FindControl(dialog, 7);
        if (affirmative == IntPtr.Zero || negative == IntPtr.Zero ||
            affirmative == negative)
            throw new InvalidOperationException("VIDEO20 native IDYES/IDNO controls missing");
        IntPtr target = yes ? affirmative : negative;
        uint owner;
        if (!IsWindowVisible(target) || !IsWindowEnabled(target) ||
            GetWindowThreadProcessId(target, out owner) == 0 ||
            owner != (uint)pid)
            throw new InvalidOperationException("VIDEO20 unowned or disabled native consent control");
        Rect rect;
        if (!GetWindowRect(target, out rect) || rect.Right-rect.Left < 6 ||
            rect.Bottom-rect.Top < 6)
            throw new InvalidOperationException("VIDEO20 native consent control has no bounds");
        Click(dialog, pid, rect.Left + (rect.Right-rect.Left)/2,
              rect.Top + (rect.Bottom-rect.Top)/2);
    }
    public static void Click(IntPtr hwnd, int pid, int x, int y) {
        uint owner;
        if (hwnd == IntPtr.Zero || GetForegroundWindow() != hwnd ||
            GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)pid)
            throw new InvalidOperationException("VIDEO20 refuses nonforeground Akula click");
        if (!SetCursorPos(x, y))
            throw new InvalidOperationException("VIDEO20 pointer move failed");
        Thread.Sleep(120);
        var hit = WindowFromPoint(new Point {X=x, Y=y});
        if (hit == IntPtr.Zero || GetAncestor(hit, 2) != hwnd ||
            GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("VIDEO20 refuses pointer outside Akula");
        var down = new Input[] {new Input {Type=0,Flags=0x0002}};
        var up = new Input[] {new Input {Type=0,Flags=0x0004}};
        if (SendInput(1, down, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("VIDEO20 real pointer down failed");
        Thread.Sleep(90);
        if (SendInput(1, up, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("VIDEO20 real pointer up failed");
    }
}
'@

$locales = @()
foreach ($lang in @('en', 'ru')) {
    $file = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $locales += ([IO.File]::ReadAllText($file, [Text.Encoding]::UTF8) | ConvertFrom-Json)
}
function Labels([scriptblock]$Accessor) {
    return @($locales | ForEach-Object { & $Accessor $_ })
}
$settingsNames = @(Labels { param($v) $v.settings.title })
$systemNames = @(Labels { param($v) $v.settings.nav.system })
$downloadNames = @(Labels { param($v) $v.settings.system.ytdlpDownload })
$removeNames = @(Labels { param($v) $v.settings.system.ytdlpRemove })
$installDialogNames = @(Labels { param($v) $v.settings.system.ytdlpConfirmTitle })
$removeDialogNames = @(Labels { param($v) $v.settings.system.ytdlpRemoveTitle })
$quickjsDownloadNames = @(Labels { param($v) $v.settings.system.quickjsDownload })
$quickjsRemoveNames = @(Labels { param($v) $v.settings.system.quickjsRemove })
$quickjsInstallDialogNames = @(Labels { param($v) $v.settings.system.quickjsConfirmTitle })
$quickjsRemoveDialogNames = @(Labels { param($v) $v.settings.system.quickjsRemoveTitle })
$buttonCondition = [Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::ControlTypeProperty,
    [Windows.Automation.ControlType]::Button)
$windowCondition = [Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::ControlTypeProperty,
    [Windows.Automation.ControlType]::Window)

function Find-Button([Windows.Automation.AutomationElement]$Root, [string[]]$Names) {
    $all = $Root.FindAll([Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    for ($i = $all.Count - 1; $i -ge 0; $i--) {
        $element = $all.Item($i)
        try {
            if (!$element.Current.IsOffscreen -and $element.Current.IsEnabled -and
                $Names -ccontains [string]$element.Current.Name) {
                return $element
            }
        } catch [Windows.Automation.ElementNotAvailableException] {}
    }
    return $null
}
function Wait-Button([Windows.Automation.AutomationElement]$Root, [string[]]$Names) {
    for ($i = 0; $i -lt 80; $i++) {
        $button = Find-Button $Root $Names
        if ($null -ne $button) { return $button }
        Start-Sleep -Milliseconds 200
    }
    throw ('Expected live Windows UIA button was not found: ' + ($Names -join ' / '))
}
function Reveal-Button([Diagnostics.Process]$Process,
    [Windows.Automation.AutomationElement]$Root, [string[]]$Names, [switch]$Up) {
    for ($n = 0; $n -lt 12; $n++) {
        $button = Find-Button $Root $Names
        if ($null -ne $button) { return $button }
        $hwnd = Foreground $Process
        $rect = $Root.Current.BoundingRectangle
        if ($rect.IsEmpty -or $rect.Width -lt 300 -or $rect.Height -lt 200) {
            throw 'VIDEO21 cannot scroll Settings without verified visible bounds'
        }
        $x = [int][Math]::Round($rect.Left + $rect.Width * 0.65)
        $y = [int][Math]::Round($rect.Top + $rect.Height * 0.60)
        [AkulaVideo20Mouse]::Scroll($hwnd, $Process.Id, $x, $y, $Up.IsPresent)
        Start-Sleep -Milliseconds 180
    }
    throw ('VIDEO20 video tool Settings button not found after native scrolling: ' + ($Names -join ' / '))
}
function Invoke-Button([Windows.Automation.AutomationElement]$Button) {
    $pattern = [Windows.Automation.InvokePattern]$Button.GetCurrentPattern(
        [Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}
function Foreground([Diagnostics.Process]$Process) {
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw 'Akula exited before opt-in GUI action'
    }
    $hwnd = [IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    for ($i = 0; $i -lt 25; $i++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
        Start-Sleep -Milliseconds 100
    }
    throw 'Akula foreground HWND could not be acquired'
}
function Press-AkulaButton([Diagnostics.Process]$Process,
    [Windows.Automation.AutomationElement]$Button) {
    $hwnd = Foreground $Process
    $rect = $Button.Current.BoundingRectangle
    if ($rect.IsEmpty -or $rect.Width -lt 4 -or $rect.Height -lt 4 -or
        $Button.Current.IsOffscreen -or !$Button.Current.IsEnabled) {
        throw 'VIDEO20 refuses hidden, disabled or zero-size settings button'
    }
    $x = [int][Math]::Round(($rect.Left + $rect.Right) / 2)
    $y = [int][Math]::Round(($rect.Top + $rect.Bottom) / 2)
    # A real physical pointer dispatch; never call Akula's Rust handlers.
    [AkulaVideo20Mouse]::Click($hwnd, $Process.Id, $x, $y)
}
function Wait-Dialog([Diagnostics.Process]$Process, [string[]]$Names) {
    for ($i = 0; $i -lt 80; $i++) {
        $top = [Windows.Automation.AutomationElement]::RootElement.FindAll(
            [Windows.Automation.TreeScope]::Children, $windowCondition)
        foreach ($window in $top) {
            try {
                if ($window.Current.ProcessId -eq $Process.Id -and
                    !$window.Current.IsOffscreen -and
                    $Names -ccontains [string]$window.Current.Name) {
                    return $window
                }
            } catch [Windows.Automation.ElementNotAvailableException] {}
        }
        Start-Sleep -Milliseconds 200
    }
    throw ('Verified Akula native consent dialog missing: ' + ($Names -join ' / '))
}
function Decide-Dialog([Diagnostics.Process]$Process,
    [Windows.Automation.AutomationElement]$Dialog, [bool]$Accept) {
    $window = [IntPtr]$Dialog.Current.NativeWindowHandle
    if ($window -eq [IntPtr]::Zero -or
        $Dialog.Current.ProcessId -ne $Process.Id) {
        throw 'VIDEO20 refuses unowned or non-native consent dialog'
    }
    # Real Win32 pointer down/up on the two exact system dialog controls.
    # No UIA caption heuristics and no direct application message/handler.
    [AkulaVideo20Mouse]::ClickConsent($window, $Process.Id, $Accept)
}
function Wait-Tool([string]$Path, [bool]$Present, [int]$Attempts) {
    for ($i = 0; $i -lt $Attempts; $i++) {
        if ((Test-Path -LiteralPath $Path -PathType Leaf) -eq $Present) { return }
        Start-Sleep -Milliseconds 250
    }
    throw "Optional yt-dlp file state mismatch (expected=$Present)"
}
function Hash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$proc = $null
New-Item -ItemType Directory -Path $VaultPath, $DataDir -Force | Out-Null
$note = Join-Path $VaultPath 'Video20 disposable note.md'
[IO.File]::WriteAllText($note, '# VIDEO20 local-only UIA note', [Text.UTF8Encoding]::new($false))
$videoDir = Join-Path $DataDir 'local-videos'
New-Item -ItemType Directory -Path $videoDir | Out-Null
$existingVideo = Join-Path $videoDir 'AbCdEfGhIJK.mp4'
$fixture = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app/native/src/video/fixtures/native_avc_aac_test.mp4'
Copy-Item -LiteralPath $fixture -Destination $existingVideo -ErrorAction Stop
$noteHash = Hash $note
$videoHash = Hash $fixture
if ((Hash $existingVideo) -cne $videoHash) { throw 'Local AVC/AAC fixture differs from the source fixture' }
$tool = Join-Path $DataDir 'tools\yt-dlp\yt-dlp-2026.10.26.exe'
$ytdlpHash = 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'
$quickjsTool = Join-Path $DataDir 'tools\quickjs\qjs.exe'

try {
    if (Test-Path -LiteralPath $tool) { throw 'Opt-in tool exists before any GUI consent' }
    if (Test-Path -LiteralPath $quickjsTool) { throw 'QuickJS exists before GUI consent' }
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $note
    $proc = Start-Process -FilePath $expectedExe -ArgumentList $args -PassThru
    $ready = $false
    for ($i = 0; $i -lt 140; $i++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw "Akula GUI exited ($($proc.ExitCode))" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like '*Akula*') { $ready = $true; break }
    }
    if (!$ready) { throw 'Native Akula window did not appear' }
    $hwnd = Foreground $proc
    $root = [Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $root) { throw 'Windows UIA root unavailable' }

    Invoke-Button (Wait-Button $root $settingsNames)
    Invoke-Button (Wait-Button $root $systemNames)
    [void](Wait-Button $root $downloadNames)
    if (Test-Path -LiteralPath $tool) { throw 'Opening Settings silently installed yt-dlp' }
    Write-Host 'VIDEO20_SETTINGS_OPEN_UIA_PASS implicit_fetch=0'

    Press-AkulaButton $proc (Wait-Button $root $downloadNames)
    Decide-Dialog $proc (Wait-Dialog $proc $installDialogNames) $false
    Start-Sleep -Milliseconds 600
    if (Test-Path -LiteralPath $tool) { throw 'Declining consent installed yt-dlp' }
    [void](Wait-Button $root $downloadNames)
    Write-Host 'VIDEO20_SETTINGS_CANCEL_UIA_PASS file_created=0'

    Press-AkulaButton $proc (Wait-Button $root $downloadNames)
    Decide-Dialog $proc (Wait-Dialog $proc $installDialogNames) $true
    Wait-Tool $tool $true 320
    if ((Get-Item -LiteralPath $tool).Length -ne 23575744 -or
        (Hash $tool) -cne $ytdlpHash) {
        throw 'Real GUI-installed yt-dlp differs from exact pinned official bytes'
    }
    [void](Wait-Button $root $removeNames)
    Write-Host 'VIDEO20_SETTINGS_INSTALL_UIA_PASS consent=1 sha256=1 local_only=1'

    # VIDEO-21: exact native mouse clicks in the same installed-style GUI.
    $quickjsButton = Reveal-Button $proc $root $quickjsDownloadNames
    if (Test-Path -LiteralPath $quickjsTool) { throw 'Opening Settings silently installed QuickJS' }
    Press-AkulaButton $proc $quickjsButton
    Decide-Dialog $proc (Wait-Dialog $proc $quickjsInstallDialogNames) $false
    Start-Sleep -Milliseconds 400
    if (Test-Path -LiteralPath $quickjsTool) { throw 'Declining QuickJS consent created an executable' }
    Write-Host 'VIDEO21_QUICKJS_CANCEL_UIA_PASS tool_created=0'

    Press-AkulaButton $proc (Reveal-Button $proc $root $quickjsDownloadNames)
    Decide-Dialog $proc (Wait-Dialog $proc $quickjsInstallDialogNames) $true
    Wait-Tool $quickjsTool $true 240
    if ((Get-Item -LiteralPath $quickjsTool).Length -ne 1873152 -or
        (Hash $quickjsTool) -cne 'ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7') {
        throw 'GUI-installed QuickJS differs from exact pinned release'
    }
    [void](Reveal-Button $proc $root $quickjsRemoveNames)
    Write-Host 'VIDEO21_QUICKJS_INSTALL_UIA_PASS consent=1 sha256=1 local_only=1'

    Press-AkulaButton $proc (Reveal-Button $proc $root $quickjsRemoveNames)
    Decide-Dialog $proc (Wait-Dialog $proc $quickjsRemoveDialogNames) $true
    Wait-Tool $quickjsTool $false 80
    if ((Hash $note) -cne $noteHash -or (Hash $existingVideo) -cne $videoHash) {
        throw 'QuickJS removal changed user Markdown or saved video'
    }
    if ((Hash $tool) -cne $ytdlpHash) { throw 'Removing QuickJS changed the installed yt-dlp' }
    Write-Host 'VIDEO21_QUICKJS_REMOVE_UIA_PASS removed=1 media_retained=1 note_unchanged=1'
    Press-AkulaButton $proc (Reveal-Button $proc $root $removeNames -Up)
    Decide-Dialog $proc (Wait-Dialog $proc $removeDialogNames) $true
    Wait-Tool $tool $false 80
    [void](Reveal-Button $proc $root $downloadNames -Up)
    if ((Hash $note) -cne $noteHash -or (Hash $existingVideo) -cne $videoHash) {
        throw 'Removing optional yt-dlp changed Markdown or local video'
    }
    if (@(Get-ChildItem -LiteralPath $VaultPath -Recurse -File |
        Where-Object { $_.Extension -in @('.mp4','.webm','.m4v','.exe') }).Count -ne 0) {
        throw 'yt-dlp or video bytes appeared in synchronized vault'
    }
    Write-Host 'VIDEO20_SETTINGS_REMOVE_UIA_PASS file_removed=1 note_unchanged=1 media_retained=1'
    Write-Host 'VIDEO22_LOCAL_MEDIA_RETENTION_PASS avc_aac_fixture=1 sha256=1 both_tools_removed=1'
}
finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (!$proc.HasExited) {
            if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(10000)) {
                $proc.Kill(); $proc.WaitForExit()
            }
        }
        $proc.Dispose()
    }
    foreach ($path in @($VaultPath, $DataDir)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
