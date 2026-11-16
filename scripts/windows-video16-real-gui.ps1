# VIDEO-16: actual Windows foreground pointer input -> installed-style staged
# Akula GUI -> real local DASH download -> restart -> decoded playback.
# Non-release, hosted-runner-only; no user vault, installer or YouTube request.
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$VaultPath,
    [Parameter(Mandatory=$true)][string]$DataDir,
    [Parameter(Mandatory=$true)][string]$ProbePath
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'VIDEO-16 may run only in GitHub-hosted Windows PowerShell 5.1 STA'
}
if ([IntPtr]::Size -ne 8) { throw 'VIDEO-16 needs x64 Windows' }
$expected = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
$tempRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
foreach($candidate in @($VaultPath, $DataDir, $ProbePath)){
    if (![IO.Path]::GetFullPath($candidate).StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase)) {
        throw 'VIDEO-16 refuses paths outside RUNNER_TEMP'
    }
}
if ([IO.Path]::GetFullPath($ExePath) -ine [IO.Path]::GetFullPath($expected) -or
    !(Test-Path -LiteralPath $expected -PathType Leaf) -or
    (Get-Process -Name akula-app -ErrorAction SilentlyContinue)) {
    throw 'VIDEO-16 needs a clean, staged Akula executable in LocalAppData'
}
foreach($folder in @($VaultPath,$DataDir)){
    if (!(Test-Path -LiteralPath $folder -PathType Container) -or
        ((Get-Item -LiteralPath $folder).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'VIDEO-16 only accepts non-aliased disposable folders'
    }
}
if ($env:AKULA_VIDEO16_PROBE -cne $ProbePath -or
    $env:AKULA_VIDEO16_MPD_URL -notmatch '^http://127[.]0[.]0[.]1:[0-9]+/manifest[.]mpd$') {
    throw 'Missing exact VIDEO-16 loopback transport and proof file'
}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class AkulaVideo16Pointer {
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(20)] public uint Flags;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }
    [DllImport("user32.dll", SetLastError=true)] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, [In] Input[] data, int size);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    public static bool IsForeground(IntPtr hwnd) { return GetForegroundWindow()==hwnd; }
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd,uint flags);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
    public static void Move(IntPtr hwnd,int pid,int x,int y) {
        uint owner;
        if (GetForegroundWindow()!=hwnd ||
            GetWindowThreadProcessId(hwnd,out owner)==0 || owner!=(uint)pid)
            throw new InvalidOperationException("VIDEO16 refused nonforeground Akula HWND/PID");
        if(!SetCursorPos(x,y)) throw new InvalidOperationException("Pointer move failed");
    }
    public static void Click(IntPtr hwnd,int pid,int x,int y) {
        Move(hwnd,pid,x,y);
        Thread.Sleep(90);
        IntPtr target=WindowFromPoint(new Point {X=x,Y=y});
        if(target==IntPtr.Zero || GetAncestor(target,2)!=hwnd || GetForegroundWindow()!=hwnd)
            throw new InvalidOperationException("VIDEO16 refuses a click outside Akula window");
        var down=new Input[] { new Input {Type=0,Flags=0x0002} };
        var up=new Input[] { new Input {Type=0,Flags=0x0004} };
        if(SendInput(1,down,Marshal.SizeOf(typeof(Input)))!=1)
            throw new InvalidOperationException("Real Windows pointer down failed");
        // Let the GUI dispatch the native down before the up, not as a
        // same-batch 0-ms pair while the editor is processing hover layout.
        Thread.Sleep(85);
        if(SendInput(1,up,Marshal.SizeOf(typeof(Input)))!=1)
            throw new InvalidOperationException("Real Windows pointer up failed");
    }
}
'@
Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop

$note = Join-Path $VaultPath 'VIDEO16 Media.md'
if (Test-Path -LiteralPath $note) { throw 'Fixture note unexpectedly exists' }
$text = "# VIDEO16 Media" + [Environment]::NewLine +
        [Environment]::NewLine +
        "https://youtu.be/dQw4w9WgXcQ" + [Environment]::NewLine
[IO.File]::WriteAllText($note,$text,[Text.UTF8Encoding]::new($false))
function Get-Video16Sha([string]$LiteralPath) {
    # Use .NET SHA256 instead of Windows PowerShell's autoloaded Utility module.
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $stream = [System.IO.File]::OpenRead($LiteralPath)
    try { return [System.BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}
$baseline = Get-Video16Sha $note
$movie = Join-Path $DataDir 'local-videos\dQw4w9WgXcQ.mp4'
if (Test-Path -LiteralPath $movie) { throw 'Fixture MP4 already present' }
if (Test-Path -LiteralPath $ProbePath) { throw 'Old GUI probe is present' }
$proc = $null

function Has-Event([string]$Name) {
    if (!(Test-Path -LiteralPath $ProbePath -PathType Leaf)) { return $false }
    return [IO.File]::ReadAllText($ProbePath).Contains(('VIDEO16_'+$Name))
}
function Start-Akula {
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath,$DataDir,$note
    $app = Start-Process -FilePath $expected -ArgumentList $args -PassThru
    for($i=0;$i -lt 140;$i++){
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if($app.HasExited){ throw "VIDEO16 installed-style process exited ($($app.ExitCode))" }
        if($app.MainWindowHandle -ne [IntPtr]::Zero -and
            $app.MainWindowTitle -like '*VIDEO16 Media*' -and
            $app.MainWindowTitle -like '*Akula*'){
            return $app
        }
    }
    throw 'VIDEO16 native Akula window was not shown'
}
function Editor-Bounds([Diagnostics.Process]$P) {
    $root=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$P.MainWindowHandle)
    if($null -eq $root){throw 'VIDEO16 missing OS UIA root'}
    $cond=[Windows.Automation.OrCondition]::new([Windows.Automation.Condition[]]@(
        [Windows.Automation.PropertyCondition]::new(
            [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit),
        [Windows.Automation.PropertyCondition]::new(
            [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Document)))
    for($attempt=0;$attempt -lt 40;$attempt++){
        $visible=@()
        $items=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$cond)
        for($i=0;$i -lt $items.Count;$i++){
            $item=$items.Item($i)
            try{
                $rect=$item.Current.BoundingRectangle
                if(!$item.Current.IsOffscreen -and $item.Current.IsEnabled -and
                   !$rect.IsEmpty -and $rect.Width -gt 300 -and $rect.Height -gt 180){
                    $visible+=,$rect
                }
            }catch [Windows.Automation.ElementNotAvailableException] {}
        }
        if($visible.Count -gt 0){
            return @($visible | Sort-Object Width -Descending)[0]
        }
        Start-Sleep -Milliseconds 200
    }
    throw 'VIDEO16 native editor is not exposed by Windows UIA'
}
function Foreground([Diagnostics.Process]$P) {
    $P.Refresh()
    if($P.HasExited){throw 'Akula exited before GUI input'}
    $hwnd=[IntPtr]$P.MainWindowHandle
    for($i=0;$i -lt 20;$i++){
        [AkulaVideo16Pointer]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        if([AkulaVideo16Pointer]::IsForeground($hwnd)){ return $hwnd }
    }
    throw 'VIDEO16 cannot focus real Akula foreground window'
}
function Find-Real-Hover([Diagnostics.Process]$P,[string]$Name) {
    $hwnd=[IntPtr]$P.MainWindowHandle
    $bounds=Editor-Bounds $P
    $left=[int][Math]::Ceiling($bounds.Left + 14)
    $right=[int][Math]::Floor($bounds.Right - 14)
    $top=[int][Math]::Ceiling($bounds.Top + 14)
    $bottom=[int][Math]::Floor($bounds.Bottom - 14)
    if($right -le $left -or $bottom -le $top){throw 'Invalid editor screen bounds'}
    # 12px is smaller than the 24px media toolbar button; the scan only
    # moves a genuine OS pointer. It cannot invoke a Rust action directly.
    $found=$false
    for($y=$top; $y -le $bottom -and !$found; $y+=12){
        for($x=$left; $x -le $right; $x+=12){
            [AkulaVideo16Pointer]::Move($hwnd,$P.Id,$x,$y)
            Start-Sleep -Milliseconds 12
            if(Has-Event $Name){
                # Mouse moves are asynchronous: click the exact Win32 pixel
                # reported by Akula's native hover hit-test, not scan position.
                $prefix='VIDEO16_'+$Name+'_POINT:'
                $record=@([IO.File]::ReadAllLines($ProbePath) |
                    Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) } |
                    Select-Object -Last 1)
                if($record.Count -ne 1){throw "VIDEO16 hover point missing for $Name"}
                $coords=$record[0].Substring($prefix.Length).Split(':')
                if($coords.Count -ne 2 -or $coords[0] -notmatch '^-?[0-9]+$' -or
                   $coords[1] -notmatch '^-?[0-9]+$'){
                    throw "VIDEO16 invalid hover point for $Name"
                }
                $px=[int]$coords[0]; $py=[int]$coords[1]
                if($px -lt $left -or $px -gt $right -or $py -lt $top -or $py -gt $bottom){
                    throw "VIDEO16 hover witness outside native editor"
                }
                [AkulaVideo16Pointer]::Move($hwnd,$P.Id,$px,$py)
                # First prove Akula actually processed a real mouse move
                # over the target button center. A hover at its edge is
                # insufficient evidence that a later click will hit.
                $centerName=if($Name -ceq 'HOVER_DOWNLOAD'){'CENTER_DOWNLOAD_READY'}else{'CENTER_LOCAL_PLAY_READY'}
                Wait-Event $centerName 30
                Start-Sleep -Milliseconds 120
                [AkulaVideo16Pointer]::Click($hwnd,$P.Id,$px,$py)
                Write-Host "VIDEO16_ACTUAL_HOVER_POINT_CLICK event=$Name center=$centerName x=$px y=$py"
                $found=$true
                break
            }
        }
    }
    if(!$found){throw "VIDEO16 real pointer could not find $Name in visible media"}
}
function Wait-Event([string]$Name,[int]$Attempts){
    for($i=0;$i -lt $Attempts;$i++){
        if(Has-Event $Name){return}
        Start-Sleep -Milliseconds 200
    }
    throw "VIDEO16 GUI event missing: $Name"
}
function Close-Akula([Diagnostics.Process]$P) {
    $P.Refresh()
    if(!$P.HasExited -and (!$P.CloseMainWindow() -or !$P.WaitForExit(20000))){
        throw 'VIDEO16 Akula did not close cleanly'
    }
    $P.Dispose()
}
try{
    $proc=Start-Akula
    # A preview must never download just from opening a note.
    Start-Sleep -Milliseconds 1200
    if(Test-Path -LiteralPath $movie){throw 'VIDEO16 implicit pre-click download'}
    $h=Foreground $proc
    Find-Real-Hover $proc 'HOVER_DOWNLOAD'
    Wait-Event 'CLICK_DOWNLOAD' 60
    $ready=$false
    for($i=0;$i -lt 480;$i++){
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if($proc.HasExited){throw 'VIDEO16 native downloader closed Akula'}
        if(Test-Path -LiteralPath $movie -PathType Leaf){
            if((Get-Item -LiteralPath $movie).Length -gt 1000){$ready=$true;break}
        }
    }
    if(!$ready){throw 'VIDEO16 physical click did not publish a device-local MP4'}
    if((Get-Video16Sha $note) -ne $baseline){
        throw 'VIDEO16 download changed synced Markdown'
    }
    if(@(Get-ChildItem -LiteralPath $VaultPath -Recurse -File |
          Where-Object { $_.Extension -in @('.mp4','.m4a','.m4s','.webm') }).Count -ne 0){
        throw 'VIDEO16 wrote a media attachment to the synchronized vault'
    }
    Write-Host 'VIDEO16_REAL_MOUSE_DOWNLOAD_PASS click=1 local_mp4=1 synced_markdown_unchanged=1'
    Close-Akula $proc; $proc=$null
    Remove-Item -LiteralPath $ProbePath -Force

    $proc=Start-Akula
    $h=Foreground $proc
    Find-Real-Hover $proc 'HOVER_LOCAL_PLAY'
    Wait-Event 'CLICK_LOCAL_PLAY' 60
    Wait-Event 'DECODED_FRAME' 150
    if((Get-Video16Sha $note) -ne $baseline){
        throw 'VIDEO16 playback after restart changed synced Markdown'
    }
    Write-Host 'VIDEO16_RESTART_DECODED_GUI_FRAME_PASS real_pointer=1 local_copy=1 decoded_frame=1'
} catch {
    if(Test-Path -LiteralPath $ProbePath -PathType Leaf) {
        Write-Host 'VIDEO16_GUI_EVENT_DIAGNOSTICS_BEGIN'
        [IO.File]::ReadAllLines($ProbePath) | Select-Object -Last 25 |
            ForEach-Object { Write-Host $_ }
        Write-Host 'VIDEO16_GUI_EVENT_DIAGNOSTICS_END'
    }
    throw
} finally {
    if($null -ne $proc){
        $proc.Refresh()
        if(!$proc.HasExited){$proc.Kill();$proc.WaitForExit()}
        $proc.Dispose()
    }
}
