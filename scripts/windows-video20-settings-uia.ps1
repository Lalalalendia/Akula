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
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }
    [DllImport("user32.dll", SetLastError=true)] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint owner);
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, [In] Input[] data, int size);
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
    # The native Yes/No button captions vary with Windows locale and may not
    # be plain "Yes"/"No" in AccessKit/UIA. Verify the system dialog has
    # exactly TWO visible enabled button rectangles, scoped to the Akula PID.
    $window = [IntPtr]$Dialog.Current.NativeWindowHandle
    if ($window -eq [IntPtr]::Zero -or
        $Dialog.Current.ProcessId -ne $Process.Id) {
        throw 'VIDEO20 refuses unowned or non-native consent dialog'
    }
    $all = $Dialog.FindAll([Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    $buttons = @()
    for ($i = 0; $i -lt $all.Count; $i++) {
        $item = $all.Item($i)
        try {
            $rect = $item.Current.BoundingRectangle
            if (!$item.Current.IsOffscreen -and $item.Current.IsEnabled -and
                !$rect.IsEmpty -and $rect.Width -gt 6 -and $rect.Height -gt 6) {
                $buttons += [pscustomobject]@{
                    element = $item
                    left = [double]$rect.Left
                    name = [string]$item.Current.Name
                }
            }
        } catch [Windows.Automation.ElementNotAvailableException] {}
    }
    if ($buttons.Count -ne 2) {
        # Only inspect the exact verified isolated dialog, never other windows.
        $seen = @($buttons | ForEach-Object { $_.name.Substring(0,[Math]::Min(40,$_.name.Length)) })
        throw ("VIDEO20 consent expects exactly two visible UIA buttons; count={0}, labels={1}" -f
            $buttons.Count, ($seen -join '|'))
    }
    $sorted = @($buttons | Sort-Object left)
    if ($sorted[0].left -ge $sorted[1].left) {
        throw 'VIDEO20 consent buttons overlap; cannot choose safely'
    }
    $yesRu = [regex]::Unescape('\u0414\u0430')
    $noRu = [regex]::Unescape('\u041d\u0435\u0442')
    $names = if ($Accept) { @('Yes', '&Yes', $yesRu, ('&' + $yesRu)) }
             else { @('No', '&No', $noRu, ('&' + $noRu)) }
    $matched = @($sorted | Where-Object { $names -ccontains $_.name })
    if ($matched.Count -gt 1) { throw 'VIDEO20 ambiguous native consent captions' }
    # With unknown OS captions, use MessageBox YesNo's verified left-to-right
    # button order on a hosted en-US runner: Yes at left, No at right.
    $choice = if ($matched.Count -eq 1) { $matched[0] }
              elseif ($Accept) { $sorted[0] }
              else { $sorted[1] }
    $rect = $choice.element.Current.BoundingRectangle
    $x = [int][Math]::Round(($rect.Left + $rect.Right) / 2)
    $y = [int][Math]::Round(($rect.Top + $rect.Bottom) / 2)
    # Recheck foreground against the SAME native dialog, not the parent Akula.
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $window) { break }
        Start-Sleep -Milliseconds 100
    }
    [AkulaVideo20Mouse]::Click($window, $Process.Id, $x, $y)
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
$existingVideo = Join-Path $videoDir 'already-downloaded.mp4'
[IO.File]::WriteAllText($existingVideo, 'existing user local media must survive yt-dlp removal')
$noteHash = Hash $note
$videoHash = Hash $existingVideo
$tool = Join-Path $DataDir 'tools\yt-dlp\yt-dlp-2026.10.26.exe'

try {
    if (Test-Path -LiteralPath $tool) { throw 'Opt-in tool exists before any GUI consent' }
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
        (Hash $tool) -cne 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d') {
        throw 'Real GUI-installed yt-dlp differs from exact pinned official bytes'
    }
    [void](Wait-Button $root $removeNames)
    Write-Host 'VIDEO20_SETTINGS_INSTALL_UIA_PASS consent=1 sha256=1 local_only=1'

    Press-AkulaButton $proc (Wait-Button $root $removeNames)
    Decide-Dialog $proc (Wait-Dialog $proc $removeDialogNames) $true
    Wait-Tool $tool $false 80
    [void](Wait-Button $root $downloadNames)
    if ((Hash $note) -cne $noteHash -or (Hash $existingVideo) -cne $videoHash) {
        throw 'Removing optional yt-dlp changed Markdown or local video'
    }
    if (@(Get-ChildItem -LiteralPath $VaultPath -Recurse -File |
        Where-Object { $_.Extension -in @('.mp4','.webm','.m4v','.exe') }).Count -ne 0) {
        throw 'yt-dlp or video bytes appeared in synchronized vault'
    }
    Write-Host 'VIDEO20_SETTINGS_REMOVE_UIA_PASS file_removed=1 note_unchanged=1 media_retained=1'
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
