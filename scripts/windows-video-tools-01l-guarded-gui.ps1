# VIDEO-TOOLS-01l: real GUI, OS pointer, protected read-only status, no process launch.
param([Parameter(Mandatory=$true)][string]$ExePath,
      [Parameter(Mandatory=$true)][string]$SentinelPath)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
   $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$' -or
   (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA -or
   $PSVersionTable.PSEdition -cne 'Desktop' -or
   [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA -or
   [IntPtr]::Size -ne 8){
   throw 'VIDEO_TOOLS_01L_HOLD: exact-head hosted Windows x64 STA PR required'
}
$who=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$who.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){
   throw 'VIDEO_TOOLS_01L_HOLD: hosted elevated ACL fixture required'
}
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
$exe=[IO.Path]::GetFullPath($ExePath)
$sentinel=[IO.Path]::GetFullPath($SentinelPath)
if(!$exe.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or
   !$sentinel.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or
   [IO.Path]::GetFileName($exe) -cne 'akula-app.exe' -or
   [IO.Path]::GetFileName($sentinel) -cne 'yt-dlp.exe' -or
   !(Test-Path -LiteralPath $exe -PathType Leaf) -or
   !(Test-Path -LiteralPath $sentinel -PathType Leaf)){
   throw 'VIDEO_TOOLS_01L_HOLD: expected clean temporary EXE and child sentinel'
}
$program=[IO.Path]::GetFullPath([Environment]::GetFolderPath('ProgramFiles'))
$root=Join-Path $program 'Akula Video Tools'
if([IO.Path]::GetFileName($root) -cne 'Akula Video Tools' -or
   [IO.Path]::GetFullPath((Split-Path $root -Parent)) -ine $program -or
   (Test-Path -LiteralPath $root) -or
   (Get-Process -Name akula-app -ErrorAction SilentlyContinue)){
   throw 'VIDEO_TOOLS_01L_HOLD: nonempty protected root or process already present'
}
$vault=Join-Path $env:RUNNER_TEMP 'video-tools-01l-vault'
$data=Join-Path $env:RUNNER_TEMP 'video-tools-01l-data'
$probe=Join-Path $env:RUNNER_TEMP 'video16-events.log'
$marker=Join-Path $env:RUNNER_TEMP 'video-tools-01l-child-executed.flag'
$note=Join-Path $vault 'VIDEO-TOOLS-01L Media.md'
$names=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe',
         'manifest.json','manifest.minisig','uninstall.exe')
foreach($path in @($vault,$data,$probe,$marker,$note)){
   if(Test-Path -LiteralPath $path){throw "VIDEO_TOOLS_01L_HOLD: preexisting fixture $path"}
}
New-Item -ItemType Directory -Path $vault -ErrorAction Stop | Out-Null
New-Item -ItemType Directory -Path $data -ErrorAction Stop | Out-Null
[IO.File]::WriteAllText($note,
  ("# VIDEO-TOOLS-01L Media"+[Environment]::NewLine+[Environment]::NewLine+
   "https://youtu.be/dQw4w9WgXcQ"+[Environment]::NewLine),
  [Text.UTF8Encoding]::new($false))
$baseline=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$env:AKULA_VIDEO16_PROBE=$probe
$env:AKULA_VIDEO_TOOLS_01L_CHILD_MARKER=$marker
$env:AKULA_YT_DLP_BIN=$sentinel
$env:AKULA_RUST_MUX_BIN=$sentinel
$env:PATH=(Split-Path $sentinel -Parent)+';'+$env:PATH
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class VideoTools01lPointer {
 [StructLayout(LayoutKind.Explicit,Size=40)]
 public struct Input {
   [FieldOffset(0)] public uint Type;
   [FieldOffset(20)] public uint Flags;
 }
 [StructLayout(LayoutKind.Sequential)]
 public struct Point { public int X; public int Y; }
 [DllImport("user32.dll",SetLastError=true)] static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,[In] Input[] data,int size);
 [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 public static bool IsForeground(IntPtr hwnd){return GetForegroundWindow()==hwnd;}
 [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd,uint flags);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
 public static void Move(IntPtr hwnd,int pid,int x,int y){
   uint owner;
   if(GetForegroundWindow()!=hwnd || GetWindowThreadProcessId(hwnd,out owner)==0 || owner!=(uint)pid)
      throw new InvalidOperationException("01l pointer owner/foreground mismatch");
   if(!SetCursorPos(x,y))throw new InvalidOperationException("01l cursor move failed");
 }
 public static void Click(IntPtr hwnd,int pid,int x,int y){
   Move(hwnd,pid,x,y); Thread.Sleep(90);
   var target=WindowFromPoint(new Point {X=x,Y=y});
   if(target==IntPtr.Zero || GetAncestor(target,2)!=hwnd || GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("01l refuses click outside Akula");
   var down=new Input[]{new Input{Type=0,Flags=0x0002}};
   var up=new Input[]{new Input{Type=0,Flags=0x0004}};
   if(SendInput(1,down,Marshal.SizeOf(typeof(Input)))!=1)throw new InvalidOperationException("01l mouse down failed");
   Thread.Sleep(85);
   if(SendInput(1,up,Marshal.SizeOf(typeof(Input)))!=1)throw new InvalidOperationException("01l mouse up failed");
 }
}
'@
function Probe-Lines {
   if(!(Test-Path -LiteralPath $probe -PathType Leaf)){return @()}
   $stream=$null;$reader=$null
   try{
      $stream=[IO.File]::Open($probe,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
      $reader=[IO.StreamReader]::new($stream,[Text.Encoding]::UTF8,$true)
      $raw=$reader.ReadToEnd()
      $end=$raw.LastIndexOf([char]10)
      if($end -lt 0){return @()}
      return @($raw.Substring(0,$end+1) -split '\r?\n' | Where-Object {$_.Length -gt 0})
   } finally {
      if($null -ne $reader){$reader.Dispose()}
      elseif($null -ne $stream){$stream.Dispose()}
   }
}
function Seen([string]$Event){return (@(Probe-Lines) -ccontains ('VIDEO16_'+$Event))}
function Wait-Event([string]$Event){
   for($n=0;$n -lt 80;$n++){
     if(Seen $Event){return}
     Start-Sleep -Milliseconds 200
   }
   throw "VIDEO_TOOLS_01L_HOLD: missing exact native event $Event"
}
function Start-Gui {
  if(Test-Path -LiteralPath $probe){Remove-Item -LiteralPath $probe -Force}
  $args='--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
  $app=Start-Process -FilePath $exe -ArgumentList $args -PassThru -ErrorAction Stop
  for($n=0;$n -lt 140;$n++){
    Start-Sleep -Milliseconds 200;$app.Refresh()
    if($app.HasExited){throw "VIDEO_TOOLS_01L_HOLD: Akula exited ($($app.ExitCode))"}
    if($app.MainWindowHandle -ne [IntPtr]::Zero -and
       $app.MainWindowTitle -like '*VIDEO-TOOLS-01L Media*' -and
       $app.MainWindowTitle -like '*Akula*'){return $app}
  }
  throw 'VIDEO_TOOLS_01L_HOLD: native Akula GUI did not appear'
}
function Focus-Gui([Diagnostics.Process]$App){
  $hwnd=[IntPtr]$App.MainWindowHandle
  for($n=0;$n -lt 20;$n++){
    [VideoTools01lPointer]::SetForegroundWindow($hwnd)|Out-Null
    Start-Sleep -Milliseconds 150
    if([VideoTools01lPointer]::IsForeground($hwnd)){return $hwnd}
  }
  throw 'VIDEO_TOOLS_01L_HOLD: foreground Akula HWND missing'
}
function Editor-Bounds([Diagnostics.Process]$App){
  $rootUi=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$App.MainWindowHandle)
  if($null -eq $rootUi){throw 'VIDEO_TOOLS_01L_HOLD: no real UIA window'}
  $condition=[Windows.Automation.OrCondition]::new([Windows.Automation.Condition[]]@(
    [Windows.Automation.PropertyCondition]::new(
       [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit),
    [Windows.Automation.PropertyCondition]::new(
       [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Document)))
  for($n=0;$n -lt 40;$n++){
    $items=$rootUi.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
    $rects=@()
    for($i=0;$i -lt $items.Count;$i++){
      try{
        $item=$items.Item($i);$rect=$item.Current.BoundingRectangle
        if(!$item.Current.IsOffscreen -and $item.Current.IsEnabled -and
           !$rect.IsEmpty -and $rect.Width -gt 300 -and $rect.Height -gt 180){$rects+=,$rect}
      }catch [Windows.Automation.ElementNotAvailableException]{}
    }
    if($rects.Count -gt 0){return @($rects|Sort-Object Width -Descending)[0]}
    Start-Sleep -Milliseconds 200
  }
  throw 'VIDEO_TOOLS_01L_HOLD: editor has no UIA rectangle'
}
function Click-Download([Diagnostics.Process]$App){
  $hwnd=Focus-Gui $App
  $r=Editor-Bounds $App
  $l=[int][Math]::Ceiling($r.Left+14);$right=[int][Math]::Floor($r.Right-14)
  $t=[int][Math]::Ceiling($r.Top+14);$bottom=[int][Math]::Floor($r.Bottom-14)
  if($right -le $l -or $bottom -le $t){throw 'VIDEO_TOOLS_01L_HOLD: invalid GUI bounds'}
  for($y=$t;$y -le $bottom;$y+=12){
    for($x=$l;$x -le $right;$x+=12){
      [VideoTools01lPointer]::Move($hwnd,$App.Id,$x,$y)
      Start-Sleep -Milliseconds 12
      if(Seen 'HOVER_DOWNLOAD'){
        $prefix='VIDEO16_HOVER_DOWNLOAD_POINT:'
        $points=@(Probe-Lines | Where-Object {$_.StartsWith($prefix,[StringComparison]::Ordinal)} |
                    Select-Object -Last 1)
        if($points.Count -ne 1){throw 'VIDEO_TOOLS_01L_HOLD: hover missing exact GUI pixel'}
        $parts=$points[0].Substring($prefix.Length).Split(':')
        if($parts.Count -ne 2 -or $parts[0] -notmatch '^-?[0-9]+$' -or
           $parts[1] -notmatch '^-?[0-9]+$'){throw 'VIDEO_TOOLS_01L_HOLD: invalid hover coordinates'}
        $px=[int]$parts[0];$py=[int]$parts[1]
        if($px -lt $l -or $px -gt $right -or $py -lt $t -or $py -gt $bottom){
          throw 'VIDEO_TOOLS_01L_HOLD: hover target outside editor'
        }
        [VideoTools01lPointer]::Move($hwnd,$App.Id,$px,$py)
        Start-Sleep -Milliseconds 120
        [VideoTools01lPointer]::Click($hwnd,$App.Id,$px,$py)
        foreach($event in @('WINDOW_POINTER_DOWN','PRESS_DOWNLOAD','CLICK_DOWNLOAD','NOTICE_BLOCKED')){
          Wait-Event $event
        }
        return
      }
    }
  }
  throw 'VIDEO_TOOLS_01L_HOLD: real mouse never found Download'
}
function Assert-Dialog([Diagnostics.Process]$App,[string]$Title){
  $uia=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$App.MainWindowHandle)
  for($n=0;$n -lt 70;$n++){
    $all=$uia.FindAll([Windows.Automation.TreeScope]::Descendants,
      [Windows.Automation.Condition]::TrueCondition)
    for($i=0;$i -lt $all.Count;$i++){
      try{
        $node=$all.Item($i)
        if($node.Current.Name -ceq $Title -and !$node.Current.IsOffscreen){return}
      }catch [Windows.Automation.ElementNotAvailableException]{}
    }
    Start-Sleep -Milliseconds 200
  }
  $names=@()
  $all=$uia.FindAll([Windows.Automation.TreeScope]::Descendants,
      [Windows.Automation.Condition]::TrueCondition)
  for($i=0;$i -lt [Math]::Min($all.Count,40);$i++){
    try{$names+=($all.Item($i).Current.Name)}catch{}
  }
  throw "VIDEO_TOOLS_01L_HOLD: AlertDialog title '$Title' absent from UIA: $($names -join ' | ')"
}
function Assert-No-Launch {
  if(Test-Path -LiteralPath $marker){throw 'VIDEO_TOOLS_01L_HOLD: child sentinel EXECUTED'}
  if(@(Get-ChildItem -LiteralPath $data -Recurse -File |
      Where-Object {$_.Extension -in @('.mp4','.m4a','.webm','.part','.m4s')}).Count -ne 0){
      throw 'VIDEO_TOOLS_01L_HOLD: blocked download created local media'
  }
  if(@(Get-ChildItem -LiteralPath $vault -Recurse -File |
      Where-Object {$_.Extension -in @('.mp4','.m4a','.webm')}).Count -ne 0){
      throw 'VIDEO_TOOLS_01L_HOLD: blocked download wrote into synced vault'
  }
  if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $baseline){
      throw 'VIDEO_TOOLS_01L_HOLD: Markdown changed after blocked GUI click'
  }
}
function Stop-Gui([Diagnostics.Process]$App){
  $App.Refresh()
  if(!$App.HasExited){
    if(!$App.CloseMainWindow() -or !$App.WaitForExit(20000)){$App.Kill();$App.WaitForExit()}
  }
  $App.Dispose()
}
function Scenario([string]$Label,[string]$Title){
  $app=$null
  try {
    $app=Start-Gui;Start-Sleep -Milliseconds 650
    Assert-No-Launch
    Click-Download $app
    Assert-Dialog $app $Title
    Start-Sleep -Milliseconds 700
    Assert-No-Launch
    Write-Host "VIDEO_TOOLS_01L_GUI_STATE_PASS state=$Label os_pointer=1 uia_dialog=1 queue_blocked=1 yt_dlp_child=0"
  } finally {
    if($null -ne $app){Stop-Gui $app}
  }
}
function Icacls([string[]]$Args){
  & "$env:WINDIR\System32\icacls.exe" @Args | Out-Host
  if($LASTEXITCODE -ne 0){throw "VIDEO_TOOLS_01L_HOLD: icacls failed $($Args -join ' ')"}
}
$created=$false
try {
  $spoof=Join-Path $env:RUNNER_TEMP 'video-tools-01l-spoof'
  New-Item -ItemType Directory -Path $spoof | Out-Null
  $env:ProgramFiles=$spoof
  Scenario 'missing' 'Akula Video Tools are not installed'
  New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
  $created=$true
  foreach($name in $names){
    [IO.File]::WriteAllBytes((Join-Path $root $name),
      [byte[]](77,90,48,49,76,45,78,79,78,82,69,76,69,65,83,69))
  }
  Icacls @($root,'/inheritance:r','/T','/C')
  Icacls @($root,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','/T','/C')
  Icacls @($root,'/setowner','*S-1-5-32-544','/T','/C')
  if(@(Get-ChildItem -LiteralPath $root -Force).Count -ne 6){
    throw 'VIDEO_TOOLS_01L_HOLD: protected fixture identity mismatch'
  }
  Scenario 'publisher-not-approved' 'Video Tools publisher is not yet approved'
  Icacls @((Join-Path $root 'akula-yt-dlp.exe'),'/grant','*S-1-5-32-545:M')
  Scenario 'unsafe' 'Video Tools installation did not pass verification'
  Write-Host 'VIDEO_TOOLS_01L_REAL_GUI_PASS real_pointer=3 uia_warning=3 no_ytdlp_child=1 no_media=1 release=0'
} finally {
  if($created -and (Test-Path -LiteralPath $root)){
    $items=@(Get-ChildItem -LiteralPath $root -Force -ErrorAction Stop)
    if($items.Count -ne 6 -or @($items|Where-Object{$_.Name -cnotin $names}).Count -ne 0){
      throw 'VIDEO_TOOLS_01L_HOLD: unexpected protected files; refusing unsafe cleanup'
    }
    Icacls @($root,'/grant','*S-1-5-32-544:(OI)(CI)F','/T','/C')
    Remove-Item -LiteralPath $root -Force -Recurse -ErrorAction Stop
    if(Test-Path -LiteralPath $root){throw 'VIDEO_TOOLS_01L_HOLD: protected fixture remained'}
  }
  if(Test-Path -LiteralPath $marker){throw 'VIDEO_TOOLS_01L_HOLD: sentinel was executed'}
}
