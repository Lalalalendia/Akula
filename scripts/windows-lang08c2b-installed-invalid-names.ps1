# LANG-08c2b1: real installed Windows GUI rejects overlong and bidi-control names.
# Scope: cold-vault manual HOLD refusal, real highlight/color, note-actions
# visibility, backup recovery, then installed study create/rename/cancel/delete.
param(
  [Parameter(Mandatory=$true)][string]$SetupPath,
  [Parameter(Mandatory=$true)][string]$ManifestPath
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or [IntPtr]::Size -ne 8 -or
    $PSVersionTable.PSEdition -cne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
  throw 'LANG-08 accepts only a GitHub-hosted x64 Windows PowerShell STA runner'
}
$workspace=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE).TrimEnd('\')+'\'
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
$setup=[IO.Path]::GetFullPath($SetupPath)
$manifestFile=[IO.Path]::GetFullPath($ManifestPath)
foreach($path in @($setup,$manifestFile)){
  if(!$path.StartsWith($workspace,[StringComparison]::OrdinalIgnoreCase) -or
     !(Test-Path -LiteralPath $path -PathType Leaf) -or
     ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)){
    throw 'LANG-08 refuses candidate outside a regular CI checkout file'
  }
}
$manifest=[IO.File]::ReadAllText($manifestFile) | ConvertFrom-Json
$setupHash=(Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
$lockHash=(Get-FileHash -LiteralPath (Join-Path $workspace 'aquilum-app\Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
if ($manifest.schema -cne 'akula-windows-beta-candidate-v1' -or
    $manifest.event_name -cne 'pull_request' -or
    $manifest.actual_checkout_sha -cne $env:GITHUB_SHA -or
    $manifest.event_sha -cne $env:GITHUB_SHA -or
    $manifest.run_id -cne $env:GITHUB_RUN_ID -or
    $manifest.run_attempt -cne $env:GITHUB_RUN_ATTEMPT -or
    $manifest.installer_name -cne [IO.Path]::GetFileName($setup) -or
    $manifest.installer_sha256 -cne $setupHash -or
    [long]$manifest.installer_bytes -ne (Get-Item -LiteralPath $setup).Length -or
    $manifest.cargo_lock_sha256 -cne $lockHash -or
    $manifest.gui_persistence_smoke -cne 'passed' -or
    $manifest.publication -cne 'CI artifact only; not a GitHub release' -or
    $manifest.signing -cne 'No Authenticode or Akula update signing performed by this workflow') {
  throw 'LANG-08 candidate provenance or non-release boundary mismatch'
}
$install=Join-Path $env:LOCALAPPDATA 'Akula'
$exe=Join-Path $install 'akula-app.exe'
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$upstream=Join-Path $env:LOCALAPPDATA 'Aquilum'
$updata=Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
$upkey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
foreach($path in @($install,$key,$upstream,$updata,$upkey)){
  if(Test-Path -LiteralPath $path){throw "LANG-08 refuses nonclean runner: $path"}
}
$vault=Join-Path $env:RUNNER_TEMP 'lang08-disposable-vault'
$data=Join-Path $env:RUNNER_TEMP 'lang08-disposable-appdata'
foreach($path in @($vault,$data)){
  if(!([IO.Path]::GetFullPath($path)).StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or
     (Test-Path -LiteralPath $path)){throw 'LANG-08 fixture paths are not fresh runner temp'}
}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class AkulaLang08Pointer {
  [StructLayout(LayoutKind.Explicit, Size=40)]
  public struct Input {
    [FieldOffset(0)] public uint Type;
    [FieldOffset(8)] public int MouseX;
    [FieldOffset(12)] public int MouseY;
    [FieldOffset(16)] public uint MouseData;
    [FieldOffset(20)] public uint MouseFlags;
    [FieldOffset(32)] public IntPtr Extra;
  }
  [DllImport("user32.dll", SetLastError=true)] static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", SetLastError=true)] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n,[In] Input[] a,int cb);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X;public int Y; }
  public static void Click(IntPtr hwnd,int pid,int x,int y,bool right){
    if(hwnd==IntPtr.Zero||Marshal.SizeOf(typeof(Input))!=40)throw new InvalidOperationException("Incorrect x64 INPUT layout");
    uint owner;
    if(GetWindowThreadProcessId(hwnd,out owner)==0||owner!=(uint)pid||GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("LANG-08 pointer refused: HWND/PID/foreground mismatch");
    if(!SetCursorPos(x,y))throw new InvalidOperationException("Cannot position mouse");
    Thread.Sleep(60);
    Point pt=new Point {X=x,Y=y};
    if(GetAncestor(WindowFromPoint(pt),2)!=hwnd || GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("LANG-08 click outside installed Akula top-level HWND");
    uint down=right?0x0008u:0x0002u, up=right?0x0010u:0x0004u;
    Input[] events=new Input[]{new Input{Type=0,MouseFlags=down},new Input{Type=0,MouseFlags=up}};
    if(SendInput(2,events,Marshal.SizeOf(typeof(Input)))!=2)
      throw new InvalidOperationException("Incomplete LANG-08 real OS click");
  }
}
'@
$labels=@{actions=@();backupExport=@();backupImport=@();rebind=@();highlight=@();yellow=@();noticeClose=@()}
foreach($lang in @('en','ru')){
  $p=Join-Path $workspace ("aquilum-app\native\assets\locales\"+$lang+".json")
  $l=[IO.File]::ReadAllText($p,[Text.Encoding]::UTF8)|ConvertFrom-Json
  foreach($k in @('actions','backupExport','backupImport','rebind','highlight','yellow','noticeClose')){
    $value=switch($k){
      actions { [string]$l.editor.noteActions }
      noticeClose { [string]$l.editor.table.studyClose }
      backupExport { [string]$l.editor.annotation.backupExport }
      backupImport { [string]$l.editor.annotation.backupImport }
      rebind { [string]$l.editor.annotation.rebindHeld }
      highlight { [string]$l.editor.annotation.highlight }
      yellow { [string]$l.editor.annotation.yellow }
    }
    if([string]::IsNullOrWhiteSpace($value)){throw "LANG-08 missing locale key $lang/$k"}
    $labels[$k]+=$value
  }
}
function Find-Visible {
  param([Windows.Automation.AutomationElement]$Root,[string[]]$Names,
        [Windows.Automation.ControlType]$Type)
  $all=$Root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  for($i=0;$i -lt $all.Count;$i++){
    try{
      $a=$all.Item($i);$s=$a.Current
      if(!$s.IsOffscreen -and $s.IsEnabled -and $Names -ccontains [string]$s.Name -and
         $s.ControlType -eq $Type -and !$s.BoundingRectangle.IsEmpty){return $a}
    }catch [Windows.Automation.ElementNotAvailableException]{}
  }
  return $null
}
function Wait-Visible {
  param([Windows.Automation.AutomationElement]$Root,[string[]]$Names,
        [Windows.Automation.ControlType]$Type)
  for($i=0;$i -lt 85;$i++){
    $found=Find-Visible -Root $Root -Names $Names -Type $Type
    if($null -ne $found){return $found}
    Start-Sleep -Milliseconds 200
  }
  $foundNames=@()
  try {
    $all=$Root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    for($j=0;$j -lt $all.Count -and $foundNames.Count -lt 30;$j++){
      try{
        $item=$all.Item($j).Current
        if(!$item.IsOffscreen -and $item.IsEnabled -and
           ($item.ControlType -eq [Windows.Automation.ControlType]::MenuItem -or
            $item.ControlType -eq [Windows.Automation.ControlType]::Button)){
          $foundNames+=("$($item.ControlType.ProgrammaticName):$($item.Name)")
        }
      }catch [Windows.Automation.ElementNotAvailableException]{}
    }
  } catch {}
  Write-Host ("LANG-08 UIA available interactive controls: "+($foundNames -join ' | '))
  throw "LANG-08 missing installed UIA control: $($Names -join ' / ')"
}
function Click-Visible {
  param([Windows.Automation.AutomationElement]$Target,
        [Windows.Automation.AutomationElement]$Window,
        [Diagnostics.Process]$Process,[switch]$Right)
  $bounds=$Target.Current.BoundingRectangle
  $win=$Window.Current.BoundingRectangle
  $x=[int][Math]::Round($bounds.Left+$bounds.Width/2)
  $y=[int][Math]::Round($bounds.Top+$bounds.Height/2)
  if($win.IsEmpty -or $bounds.IsEmpty -or $bounds.Width -lt 4 -or $bounds.Height -lt 4 -or
     $x -lt $win.Left -or $x -ge $win.Right -or $y -lt $win.Top -or $y -ge $win.Bottom){
    throw 'LANG-08 UIA target outside visible Akula window'
  }
  [AkulaLang08Pointer]::Click([IntPtr]$Process.MainWindowHandle,$Process.Id,$x,$y,$Right.IsPresent)
}
function Wait-Window {
  param([Diagnostics.Process]$Process,[string]$Needle)
  for($i=0;$i -lt 100;$i++){
    Start-Sleep -Milliseconds 200
    $Process.Refresh()
    if($Process.HasExited){throw 'Installed Akula exited during LANG-08'}
    if($Process.MainWindowHandle -ne [IntPtr]::Zero -and
       $Process.MainWindowTitle.Contains($Needle) -and
       $Process.MainWindowTitle.Contains('Akula')){return}
  }
  throw 'LANG-08 installed Akula did not display the fixture note'
}
function Focus-Akula {
  param([Diagnostics.Process]$Process)
  $hwnd=[IntPtr]$Process.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd,9)|Out-Null
  for($i=0;$i -lt 20;$i++){
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd)|Out-Null
    Start-Sleep -Milliseconds 150
    if([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd){return}
  }
  throw 'Cannot focus the exact installed Akula window'
}
function Select-EditorBody {
  param([Windows.Automation.AutomationElement]$Root)
  $all=$Root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  $found=@()
  $win=$Root.Current.BoundingRectangle
  for($i=0;$i -lt $all.Count;$i++){
    try {
      $a=$all.Item($i);$s=$a.Current;$b=$s.BoundingRectangle
      if(!$s.IsEnabled -or !$s.IsKeyboardFocusable -or $s.IsOffscreen -or $b.IsEmpty -or
         $b.Width -lt 160 -or $b.Height -lt 10 -or $b.Left -lt $win.Left -or
         $b.Right -gt $win.Right -or $b.Top -lt $win.Top -or $b.Bottom -gt $win.Bottom){continue}
      if($s.ControlType -eq [Windows.Automation.ControlType]::Edit -or
         $s.ControlType -eq [Windows.Automation.ControlType]::Document){$found+=$a}
    }catch [Windows.Automation.ElementNotAvailableException]{}
  }
  if($found.Count -eq 0){throw 'LANG-08 missing visible installed Markdown editor'}
  $body=@($found | Sort-Object {$_.Current.BoundingRectangle.Top})[-1]
  $body.SetFocus()
  return $body
}
# A top-level title can appear before Masonry exposes the Markdown editor
# in the Windows UIA tree. Re-acquire the real installed HWND on each attempt,
# require an enabled, visible, focusable Edit/Document control, and fail closed.
function Wait-EditorBody {
  param([Diagnostics.Process]$Process,[string]$Needle)
  $lastWindow=$null
  for($attempt=0;$attempt -lt 90;$attempt++){
    $Process.Refresh()
    if($Process.HasExited){throw 'LANG-08 installed Akula exited while opening the Markdown editor'}
    if($Process.MainWindowHandle -ne [IntPtr]::Zero -and
       $Process.MainWindowTitle.Contains('Akula') -and
       $Process.MainWindowTitle.Contains($Needle)){
      try {
        $lastWindow=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Process.MainWindowHandle)
        if($null -ne $lastWindow){
          $editor=Select-EditorBody -Root $lastWindow
          return @{ Window=$lastWindow; Editor=$editor }
        }
      } catch {
        # Only the known transient no-editor state is retried.
        if($_.Exception.Message -cne 'LANG-08 missing visible installed Markdown editor'){throw}
      }
    }
    Start-Sleep -Milliseconds 200
  }
  $visible=@()
  if($null -ne $lastWindow){
    try {
      $all=$lastWindow.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
      for($i=0;$i -lt $all.Count -and $visible.Count -lt 35;$i++){
        try {
          $node=$all.Item($i).Current
          if(!$node.IsOffscreen){
            $visible+=("$($node.ControlType.ProgrammaticName):$($node.Name)")
          }
        } catch [Windows.Automation.ElementNotAvailableException] {}
      }
    } catch {}
  }
  Write-Host ("LANG-08 restart UIA visible controls: "+($visible -join ' | '))
  throw 'LANG-08 installed editor did not become visible and focusable after restart'
}
function Readback-Selected {
  param([Diagnostics.Process]$Process,[string]$Expected)
  $hwnd=[IntPtr]$Process.MainWindowHandle
  $poison='LANG08_POISON_'+[Guid]::NewGuid().ToString('N')
  [Windows.Forms.Clipboard]::SetText($poison)
  [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd,$Process.Id,[uint16]0x41)
  Start-Sleep -Milliseconds 180
  [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd,$Process.Id,[uint16]0x43)
  $copy=$null
  for($i=0;$i -lt 70;$i++){
    Start-Sleep -Milliseconds 130
    try{
      $v=[Windows.Forms.Clipboard]::GetText()
      if($v -cne $poison -and ![string]::IsNullOrEmpty($v)){$copy=$v;break}
    }catch{}
  }
  $normalize={param([string]$Text)
    $lf=[string][char]10;$cr=[string][char]13
    return $Text.Replace($cr+$lf,$lf).Replace($cr,$lf).TrimEnd([char]10)
  }
  if($null -eq $copy -or
     ![string]::Equals((& $normalize $copy),(& $normalize $Expected),[StringComparison]::Ordinal)){
    $reason=if($null -eq $copy){'clipboard_copy_missing'}else{'text_mismatch'}
    $observedLength=if($null -eq $copy){0}else{(& $normalize $copy).Length}
    $expectedLength=(& $normalize $Expected).Length
    throw "LANG-08 installed editor failed exact OS Ctrl+A/C clipboard readback reason=$reason expected_length=$expectedLength observed_length=$observedLength"
  }
}
# SQLite WAL may checkpoint without changing records: compare snapshot instead
# of physical database bytes, while still requiring a genuine SQLite file.
function Read-AnnotationState {
  param([string]$Path)
  # Windows PowerShell 5.1 can strip quotes from python -c arguments.
  # Feed the fixed, ASCII-only script via stdin instead; SQL and data paths
  # stay separate arguments, and no transient Python file is created.
  $py=@'
import hashlib
import pathlib
import sqlite3
import sys
db = pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(db.as_uri() + "?mode=ro", uri=True) as con:
    row = con.execute("SELECT revision, payload FROM text_annotation_state WHERE id=1").fetchone()
if row is None or row[0] < 0 or not isinstance(row[1], bytes):
    raise ValueError("Invalid annotation snapshot")
print(str(row[0]) + ":" + hashlib.sha256(row[1]).hexdigest())
'@
  $result=($py | & python - $Path | Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or $result -notmatch '^[0-9]+:[a-f0-9]{64}$'){
    throw 'LANG-08 could not read a valid SQLite annotation snapshot'
  }
  return $result
}
function Open-Note {
  param([string]$Note,[string]$Vault,[string]$Data)
  $arguments='--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $Vault,$Data,$Note
  return Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
}
function Close-Note {
  param([Diagnostics.Process]$Process)
  if(!$Process.CloseMainWindow() -or !$Process.WaitForExit(20000)){
    throw 'LANG-08 installed Akula refused graceful close'
  }
  $Process.Dispose()
}
# LANG-08b uses native Windows file-picker UIA, not a mocked RFD response.
# The dialog must belong to the exact installed Akula PID and own foreground.
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaLang08DialogKeys {
  [StructLayout(LayoutKind.Explicit, Size=40)]
  public struct Input {
    [FieldOffset(0)] public uint Type;
    [FieldOffset(8)] public ushort Key;
    [FieldOffset(12)] public uint Flags;
    [FieldOffset(24)] public IntPtr Extra;
  }
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", SetLastError=true)]
  static extern uint GetWindowThreadProcessId(IntPtr h,out uint processId);
  [DllImport("user32.dll",SetLastError=true)]
  static extern uint SendInput(uint n,[In] Input[] inputs,int size);
  // Only Ctrl+A, Ctrl+V and Ctrl+Z on the verified foreground installed
  // Akula PID. Ctrl+Z restores the single disposable unsaved Markdown paste
  // after the study refusal, including when its table cell has focus.
  // Never arbitrary keyboard/window injection.
  public static void Chord(IntPtr hwnd,int expectedPid,ushort key) {
    if(key != 0x41 && key != 0x56 && key != 0x5A)
      throw new InvalidOperationException("Unapproved installed synthetic editor key");
    uint pid;
    if(hwnd==IntPtr.Zero || Marshal.SizeOf(typeof(Input))!=40 ||
       GetWindowThreadProcessId(hwnd,out pid)==0 || pid!=(uint)expectedPid ||
       GetForegroundWindow()!=hwnd)throw new InvalidOperationException("File-picker PID/foreground mismatch");
    const uint UP=2;
    Input[] keys=new Input[]{
      new Input{Type=1,Key=0x11},
      new Input{Type=1,Key=key},
      new Input{Type=1,Key=key,Flags=UP},
      new Input{Type=1,Key=0x11,Flags=UP}
    };
    if(SendInput(4,keys,Marshal.SizeOf(typeof(Input)))!=4)
      throw new InvalidOperationException("Incomplete file-picker SendInput");
  }
  // Windows common Save/Open file dialogs have a File name mnemonic Alt+N.
  // On the hosted runner the input is sometimes absent from the UIA Edit
  // tree while the genuine #32770 dialog itself is visible. Restrict this
  // access-key fallback to the foreground dialog owned by installed Akula.
  public static void FocusFileName(IntPtr hwnd,int expectedPid) {
    uint pid;
    if(hwnd==IntPtr.Zero || Marshal.SizeOf(typeof(Input))!=40 ||
       GetWindowThreadProcessId(hwnd,out pid)==0 || pid!=(uint)expectedPid ||
       GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("File-picker access key PID/foreground mismatch");
    const uint UP=2;
    Input[] keys=new Input[]{
      new Input{Type=1,Key=0x12}, // VK_MENU (Alt)
      new Input{Type=1,Key=0x4e}, // VK_N (English File name mnemonic)
      new Input{Type=1,Key=0x4e,Flags=UP},
      new Input{Type=1,Key=0x12,Flags=UP}
    };
    if(SendInput(4,keys,Marshal.SizeOf(typeof(Input)))!=4)
      throw new InvalidOperationException("Incomplete file-picker filename access key");
  }
  // The shared AccessKit helper deliberately forbids ESC: its whitelist is
  // for the global GUI acceptance. This separate, narrow table-menu cleanup
  // only sends one Escape to the exact foreground Akula HWND and PID.
  public static void DismissTableProbe(IntPtr hwnd,int expectedPid) {
    uint pid;
    if(hwnd==IntPtr.Zero || Marshal.SizeOf(typeof(Input))!=40 ||
       GetWindowThreadProcessId(hwnd,out pid)==0 || pid!=(uint)expectedPid ||
       GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("Table-probe Escape PID/foreground mismatch");
    const uint UP=2;
    Input[] keys=new Input[]{
      new Input{Type=1,Key=0x1b},
      new Input{Type=1,Key=0x1b,Flags=UP}
    };
    if(SendInput(2,keys,Marshal.SizeOf(typeof(Input)))!=2)
      throw new InvalidOperationException("Incomplete table-probe Escape input");
  }
}
'@
function Wait-FileDialog {
  param([Diagnostics.Process]$Process,[IntPtr]$AppWindowHandle)
  $desktop=[Windows.Automation.AutomationElement]::RootElement
  $windowType=[Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::ControlTypeProperty,
    [Windows.Automation.ControlType]::Window)
  for($attempt=0;$attempt -lt 120;$attempt++){
    $Process.Refresh()
    if($Process.HasExited){throw 'LANG-08b Akula exited while opening native file picker'}
    $windows=$desktop.FindAll([Windows.Automation.TreeScope]::Children,$windowType)
    for($i=0;$i -lt $windows.Count;$i++){
      try {
        $w=$windows.Item($i);$s=$w.Current
        if($s.ProcessId -eq $Process.Id -and !$s.IsOffscreen -and $s.IsEnabled -and
           $s.NativeWindowHandle -ne 0 -and
           [long]$s.NativeWindowHandle -ne $AppWindowHandle.ToInt64() -and
           $s.ClassName -ceq '#32770' -and !$s.BoundingRectangle.IsEmpty){
          return $w
        }
      } catch [Windows.Automation.ElementNotAvailableException] {}
    }
    Start-Sleep -Milliseconds 200
  }
  $windows=$desktop.FindAll([Windows.Automation.TreeScope]::Children,$windowType)
  $seen=@()
  for($i=0;$i -lt $windows.Count -and $seen.Count -lt 25;$i++){
    try{
      $s=$windows.Item($i).Current
      if($s.ProcessId -eq $Process.Id){
        $seen+=("$($s.ClassName):$($s.Name):$($s.NativeWindowHandle)")
      }
    }catch [Windows.Automation.ElementNotAvailableException] {}
  }
  Write-Host ("LANG-08b installed PID windows: "+($seen -join ' | '))
  throw 'LANG-08b genuine native Akula file-picker window was not observable'
}
function Choose-FileThroughNativeDialog {
  param([Diagnostics.Process]$Process,[string]$Path,[IntPtr]$AppWindowHandle)
  $target=[IO.Path]::GetFullPath($Path)
  if(!$target.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or
     [IO.Path]::GetExtension($target) -cne '.json' -or
     [IO.Path]::GetFileName($target) -cne 'lang08b-disposable.annotations.json'){
    throw 'LANG-08b refuses any file-picker destination other than its synthetic JSON'
  }
  $dialog=Wait-FileDialog -Process $Process -AppWindowHandle $AppWindowHandle
  $hwnd=[IntPtr]$dialog.Current.NativeWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd,9)|Out-Null
  for($n=0;$n -lt 25;$n++){
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd)|Out-Null
    Start-Sleep -Milliseconds 120
    if([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd){break}
  }
  if([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd){
    throw 'LANG-08b could not foreground verified Akula file picker'
  }
  $all=$dialog.FindAll([Windows.Automation.TreeScope]::Descendants,
      [Windows.Automation.Condition]::TrueCondition)
  $filename=$null;$seen=@()
  for($i=0;$i -lt $all.Count;$i++){
    try{
      $e=$all.Item($i);$s=$e.Current
      if($s.ControlType -ne [Windows.Automation.ControlType]::Edit -or
         !$s.IsEnabled -or $s.IsOffscreen -or $s.BoundingRectangle.IsEmpty){continue}
      $seen+=("$($s.AutomationId):$($s.Name)")
      if($s.AutomationId -ceq '1148' -or
         $s.Name -match '^(File name|Имя файла)(:)?$'){
        $filename=$e;break
      }
    }catch [Windows.Automation.ElementNotAvailableException] {}
  }
  if($null -eq $filename){
    # The genuine Windows common-file dialog can expose the shell list and
    # Search Box but omit File name as a visible UIA Edit until focused.
    # Use the standard English File name access key through real OS input,
    # never substitute a synthetic RFD callback or edit application data.
    Write-Host 'LANG-08b native File name Edit unavailable in UIA; using PID-checked OS Alt+N'
    [AkulaLang08DialogKeys]::FocusFileName($hwnd,$Process.Id)
    Start-Sleep -Milliseconds 150
  } else {
    $rect=$filename.Current.BoundingRectangle
    $x=[int][Math]::Round($rect.Left+$rect.Width/2)
    $y=[int][Math]::Round($rect.Top+$rect.Height/2)
    [AkulaLang08Pointer]::Click($hwnd,$Process.Id,$x,$y,$false)
  }
  if([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd){
    throw 'LANG-08b file picker lost verified foreground before input'
  }
  [Windows.Forms.Clipboard]::SetText($target)
  if([Windows.Forms.Clipboard]::GetText() -cne $target){
    throw 'LANG-08b synthetic clipboard filename did not round-trip'
  }
  [AkulaLang08DialogKeys]::Chord($hwnd,$Process.Id,[uint16]0x41)
  [AkulaLang08DialogKeys]::Chord($hwnd,$Process.Id,[uint16]0x56)
  Start-Sleep -Milliseconds 200
  [AkulaBetaGuiInput]::SendNavigationKey($hwnd,$Process.Id,[uint16]0x0d,$false)
}
# This witness reads SQLite only. No bypass into the Akula application.
function Get-AnnotationSnapshot {
  param([string]$Path)
  $reader=@'
import json
import pathlib
import sqlite3
import sys
db=pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(db.as_uri()+"?mode=ro",uri=True) as conn:
    row=conn.execute("SELECT revision,payload FROM text_annotation_state WHERE id=1").fetchone()
if row is None or row[0]<0:
    raise ValueError("Missing annotation revision")
payload=json.loads(row[1])
records=payload["records"]
if len(records)>1:
    raise ValueError("Unexpected second synthetic annotation")
record=records[0] if records else None
digest=None
if record is not None:
    values=record["anchor"]["selected_digest"]
    if (not isinstance(values,list) or len(values)!=32 or
        any(type(value) is not int or value<0 or value>255 for value in values)):
        raise ValueError("Invalid serialized 32-byte selected_digest")
    digest=bytes(values).hex()
print(json.dumps({"revision":int(row[0]),"record_count":len(records),
    "status":record["status"] if record else None,
    "annotation_id":record["anchor"]["annotation_id"] if record else None,
    "selected_digest":digest,
    "color":record["color"] if record else None}))
'@
  $out=($reader | & python - $Path | Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or !$out){throw 'LANG-08b could not read real annotation SQLite payload'}
  return ($out | ConvertFrom-Json)
}

# LANG-08c reads only the app-owned synthetic SQLite study registry.
function Get-StudySnapshot {
  param([string]$Path)
  $reader=@'
import json
import pathlib
import sqlite3
import sys
db=pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(db.as_uri()+"?mode=ro",uri=True) as c:
    row=c.execute("SELECT revision,payload FROM language_study_state WHERE id=1").fetchone()
if row is None or row[0]<0:
    raise ValueError("Missing language-study SQLite revision")
p=json.loads(row[1])
if p.get("version")!=1 or "state" not in p:
    raise ValueError("Unexpected language-study payload schema")
reg=p["state"]["registry"]
groups=reg["collections"]
if len(groups)>1:
    raise ValueError("LANG-08c synthetic fixture expects no unrelated collections")
g=groups[0] if groups else None
ids=[x["id"] for x in groups]
if len(ids)!=len(set(ids)):
    raise ValueError("Duplicate language-study collection ID")
print(json.dumps({"revision":int(row[0]),"count":len(groups),
    "id":g["id"] if g else None, "name":g["name"] if g else None,
    "members":len(g["members"]) if g else 0,
    "table_count":len(reg["tables"]),
    "anchor_count":len(p["state"]["anchors"])}))
'@
  $out=($reader | & python - $Path | Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or !$out){throw 'LANG-08c cannot read real SQLite study state'}
  return ($out | ConvertFrom-Json)
}
function Study-Choose {
  param([Windows.Automation.AutomationElement]$Window,
        [Diagnostics.Process]$Process,[string[]]$Names)
  $item=Wait-Visible -Root $Window -Names $Names -Type ([Windows.Automation.ControlType]::MenuItem)
  Click-Visible -Target $item -Window $Window -Process $Process
}
function Open-StudyTableMenu {
  param([Windows.Automation.AutomationElement]$Window,
        [Diagnostics.Process]$Process)
  Focus-Akula -Process $Process
  $editor=Select-EditorBody -Root $Window
  $bounds=$editor.Current.BoundingRectangle
  $hwnd=[IntPtr]$Process.MainWindowHandle
  $attempts=0
  foreach($yoff in @(25,42,60,79,100,125,150,185,220,260,300,340)){
    foreach($xshare in @(0.3,0.5,0.7)){
      $x=[int][Math]::Round($bounds.Left+$bounds.Width*$xshare)
      $y=[int][Math]::Round($bounds.Top+$yoff)
      if($x -ge $bounds.Right -or $y -ge $bounds.Bottom -or
         $y -le $bounds.Top -or $x -le $bounds.Left){continue}
      $attempts++
      # Probe real visible table cells. A plain editor context menu is not
      # accepted; only native "Add to study collection" proves TableMenu.
      [AkulaLang08Pointer]::Click($hwnd,$Process.Id,$x,$y,$true)
      Start-Sleep -Milliseconds 120
      $add=Find-Visible -Root $Window -Names $labels.studyAdd -Type ([Windows.Automation.ControlType]::MenuItem)
      if($null -ne $add){return $add}
      [AkulaLang08DialogKeys]::DismissTableProbe($hwnd,$Process.Id)
      Start-Sleep -Milliseconds 90
    }
  }
  throw "LANG-08c no real TableMenu at any of $attempts bounded OS clicks (editor bounds=$bounds)"
}
function Open-StudyCollectionMenu {
  param([Windows.Automation.AutomationElement]$Window,
        [Diagnostics.Process]$Process)
  $add=Open-StudyTableMenu -Window $Window -Process $Process
  Click-Visible -Target $add -Window $Window -Process $Process
  # powershell.exe 5.1 -File decodes UTF-8 without BOM as legacy ANSI.
  # Build the exact U+00B7 separator from ASCII-only source to match UIA.
  $middot=[char]0x00b7
  Study-Choose -Window $Window -Process $Process -Names @("1 $middot Q")
  Study-Choose -Window $Window -Process $Process -Names @("2 $middot A")
}
function Type-StudyName {
  param([Diagnostics.Process]$Process,[string]$Name)
  if([string]::IsNullOrWhiteSpace($Name) -or $Name.Length -gt 64 -or
     $Name -notmatch '^LANG08C-[A-Za-z-]+$'){
    throw 'LANG-08c refuses non-fixture collection names'
  }
  # The native Create/Rename dialog focuses the dedicated name editor on
  # opening. Only physical OS shortcuts can supply the name, not StateStore.
  $hwnd=[IntPtr]$Process.MainWindowHandle
  if([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd){
    throw 'LANG-08c fixture name dialog lost foreground Akula HWND'
  }
  [Windows.Forms.Clipboard]::SetText($Name)
  if([Windows.Forms.Clipboard]::GetText() -cne $Name){
    throw 'LANG-08c cannot set the disposable name clipboard'
  }
  [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd,$Process.Id,[uint16]0x41)
  [AkulaLang08DialogKeys]::Chord($hwnd,$Process.Id,[uint16]0x56)
}
# LANG-08c2b1: exact synthetic negative names only. Never arbitrary input.
function Type-StudyInvalidName {
  param([Diagnostics.Process]$Process,
        [ValidateSet('too-long','bidi')][string]$Case)
  if($Case -ceq 'too-long'){
    $name='LANG08C-'+('X'*65)
    if($name.Length -le 64){throw 'LANG-08c2b overlong fixture invalid'}
  } elseif($Case -ceq 'bidi') {
    $name='LANG08C-'+[char]0x202e+'Abc'
    if(!$name.Contains([char]0x202e)){throw 'LANG-08c2b bidi fixture invalid'}
  } else {
    throw 'Unapproved LANG-08c2b negative fixture case'
  }
  $hwnd=[IntPtr]$Process.MainWindowHandle
  if($hwnd -eq [IntPtr]::Zero -or [AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd){
    throw 'LANG-08c2b requires foreground installed Akula'
  }
  [Windows.Forms.Clipboard]::SetText($name)
  if([Windows.Forms.Clipboard]::GetText() -cne $name){
    throw 'LANG-08c2b synthetic name clipboard mismatch'
  }
  [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd,$Process.Id,[uint16]0x41)
  [AkulaLang08DialogKeys]::Chord($hwnd,$Process.Id,[uint16]0x56)
}
function Close-StudyNotice {
  param([Windows.Automation.AutomationElement]$Window,
        [Diagnostics.Process]$Process)
  $close=Wait-Visible -Root $Window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $close -Window $Window -Process $Process
  Start-Sleep -Milliseconds 400
}
function Require-StudySnapshot {
  # [string] coerces a null argument into an empty string on PowerShell 5.1,
  # but the genuine Rust/serde empty registry returns JSON null for name.
  # Preserve null and assert that deletion truly removed the collection.
  param([string]$Path,[int]$Count,[AllowNull()][object]$Name)
  for($i=0;$i -lt 70;$i++){
    Start-Sleep -Milliseconds 150
    if(!(Test-Path -LiteralPath $Path -PathType Leaf)){continue}
    $snap=Get-StudySnapshot -Path $Path
    $matchesName=if($Count -eq 0) {
      ($null -eq $snap.name -and $null -eq $Name)
    } else {
      ($null -ne $snap.name -and $snap.name -ceq $Name)
    }
    if($snap.count -eq $Count -and $matchesName){return $snap}
  }
  throw "LANG-08c expected installed UI SQLite named collection $Name/$Count"
}


function Read-StudyStateHash {
  param([string]$Path)
  $code=@'
import hashlib
import pathlib
import sqlite3
import sys
db=pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(db.as_uri()+"?mode=ro",uri=True) as c:
    row=c.execute("SELECT revision,payload FROM language_study_state WHERE id=1").fetchone()
if row is None or row[0]<0:
    raise ValueError("Missing language-study SQLite payload")
print(str(row[0])+":"+hashlib.sha256(row[1]).hexdigest())
'@
  $out=($code | & python - $Path | Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or $out -notmatch '^\d+:[a-f0-9]{64}$'){
    throw 'LANG-08c invalid study SQLite revision hash'
  }
  return $out
}

function Assert-SyntheticUnchanged {
  param([string]$Markdown,[string]$ExpectedHash,[string]$Backup,[string]$BackupHash)
  if((Get-FileHash -LiteralPath $Markdown -Algorithm SHA256).Hash -cne $ExpectedHash -or
     (Get-FileHash -LiteralPath $Backup -Algorithm SHA256).Hash -cne $BackupHash){
    throw 'LANG-08b modified synthetic source Markdown or exported JSON backup'
  }
}


foreach($lang in @('en','ru')){
  $p=Join-Path $workspace ("aquilum-app\native\assets\locales\"+$lang+".json")
  $l=[IO.File]::ReadAllText($p,[Text.Encoding]::UTF8)|ConvertFrom-Json
  foreach($k in @('studyAdd','studyNew','studyCreate','studyManage','studyRenameSave','studyDeleteConfirm')){
    $value=[string]$l.editor.table.$k
    if([string]::IsNullOrWhiteSpace($value)){throw "LANG-08c missing native locale $lang/$k"}
    if(!$labels.ContainsKey($k)){$labels[$k]=@()}
    $labels[$k]+=$value
  }
}

$installed=$false
$proc=$null
$note=$null
$beforeHash=$null
$dbState=$null
New-Item -ItemType Directory -Path $vault,$data -ErrorAction Stop|Out-Null
New-Item -ItemType Directory -Path $upstream,$updata -ErrorAction Stop|Out-Null
[IO.File]::WriteAllText((Join-Path $upstream 'sentinel.txt'),'AKULA_LANG08_UPSTREAM')
[IO.File]::WriteAllText((Join-Path $updata 'sentinel.txt'),'AKULA_LANG08_UPDATA')
New-Item -Path $upkey -Force|Out-Null
Set-ItemProperty -Path $upkey -Name DisplayName -Value 'Aquilum'
function Assert-Upstream {
  if([IO.File]::ReadAllText((Join-Path $upstream 'sentinel.txt')) -cne 'AKULA_LANG08_UPSTREAM' -or
     [IO.File]::ReadAllText((Join-Path $updata 'sentinel.txt')) -cne 'AKULA_LANG08_UPDATA' -or
     (Get-ItemProperty $upkey).DisplayName -cne 'Aquilum') {
    throw 'LANG-08 installed Akula altered a simulated upstream Aquilum installation'
  }
}
try {
  $setupProc=Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
  if($setupProc.ExitCode -ne 0){throw "LANG-08 NSIS setup failed ($($setupProc.ExitCode))"}
  $installed=$true
  $installedHash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
  if(!(Test-Path -LiteralPath $key) -or
     (Get-ItemProperty $key).DisplayName -cne 'Akula' -or
     $installedHash -cne $manifest.compiled_executable_sha256) {
    throw 'LANG-08 wrong installed NSIS identity or compiled EXE SHA256'
  }
  Assert-Upstream
  $note=Join-Path $vault 'LANG08 Annotation.md'
  $text='# LANG08 Annotation'+[char]10+[char]10+'alpha eat omega'+[char]10
  [IO.File]::WriteAllText($note,$text,[Text.UTF8Encoding]::new($false))
  $beforeHash=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
  $db=Join-Path $data 'text-annotations.sqlite'
  if(Test-Path -LiteralPath $db){throw 'LANG-08 cold sidecar was unexpectedly present'}
  $proc=Open-Note -Note $note -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08 Annotation'
  Focus-Akula -Process $proc
  $window=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
  if($null -eq $window){throw 'LANG-08 installed window has no UIA tree'}
  $body=Select-EditorBody -Root $window
  Readback-Selected -Process $proc -Expected $text

  # Negative: exact selection + real context menu + rebind with no sidecar.
  Click-Visible -Target $body -Window $window -Process $proc -Right
  $reb=Wait-Visible -Root $window -Names $labels.rebind -Type ([Windows.Automation.ControlType]::MenuItem)
  Click-Visible -Target $reb -Window $window -Process $proc
  if(Test-Path -LiteralPath $db){throw 'LANG-08 invalid HOLD rebind created a database'}
  if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $beforeHash){
    throw 'LANG-08 invalid rebind modified Markdown'
  }
  # The failed rebind deliberately raises a *modal* study_notice (AlertDialog).
  # Do not try to interact with the obscured editor until we close it through
  # the actual installed Windows UI. This modal is part of the negative test.
  $closeNotice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $closeNotice -Window $window -Process $proc
  # UIAutomation can retain a stale exposed Close button after the native
  # modal begins its fade-out. Presence of that label in the broad root tree
  # is NOT evidence the dialog still owns focus. Verify real behavior instead:
  # regain the editor, read back exact Markdown with physical Ctrl+A/C and
  # prove that its actual context menu is openable through the OS mouse.
  Start-Sleep -Milliseconds 400
  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  Readback-Selected -Process $proc -Expected $text
  Click-Visible -Target $body -Window $window -Process $proc -Right
  $highlight=Wait-Visible -Root $window -Names $labels.highlight -Type ([Windows.Automation.ControlType]::MenuItem)
  Write-Host 'AKULA_LANG08_INSTALLED_REBIND_DENY_PASS genuine_nsis=1 os_mouse=1 sidecar_created=0 source_unchanged=1 editor_access_restored=1'
  Click-Visible -Target $highlight -Window $window -Process $proc
  $yellow=Wait-Visible -Root $window -Names $labels.yellow -Type ([Windows.Automation.ControlType]::MenuItem)
  Click-Visible -Target $yellow -Window $window -Process $proc
  $dbReady=$false
  for($i=0;$i -lt 80;$i++){
    Start-Sleep -Milliseconds 150
    if(Test-Path -LiteralPath $db -PathType Leaf){
      $info=Get-Item -LiteralPath $db
      if($info.Length -gt 512){$dbReady=$true;break}
    }
  }
  if(!$dbReady){throw 'LANG-08 real OS highlight never persisted SQLite'}
  $header=[IO.File]::ReadAllBytes($db)
  if([Text.Encoding]::ASCII.GetString($header,0,15) -cne 'SQLite format 3'){
    throw 'LANG-08 native annotation sidecar is not a SQLite database'
  }
  if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $beforeHash){
    throw 'LANG-08 highlight modified source Markdown'
  }
  Write-Host 'AKULA_LANG08_INSTALLED_HIGHLIGHT_PASS genuine_nsis=1 real_color_menu=1 sqlite=1 source_unchanged=1'

  Focus-Akula -Process $proc
  $actions=Wait-Visible -Root $window -Names $labels.actions -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $actions -Window $window -Process $proc
  [void](Wait-Visible -Root $window -Names $labels.backupExport -Type ([Windows.Automation.ControlType]::MenuItem))
  [void](Wait-Visible -Root $window -Names $labels.backupImport -Type ([Windows.Automation.ControlType]::MenuItem))
  Write-Host 'AKULA_LANG08_INSTALLED_BACKUP_MENU_PASS genuine_nsis=1 export_visible=1 import_visible=1'
  Close-Note -Process $proc;$proc=$null
  $dbState=Read-AnnotationState -Path $db
  if($dbState -notmatch '^[1-9][0-9]*:[a-f0-9]{64}$'){
    throw 'LANG-08 installed highlight missing positive SQLite revision and payload digest'
  }

  $proc=Open-Note -Note $note -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08 Annotation'
  Focus-Akula -Process $proc
  $reopened=Wait-EditorBody -Process $proc -Needle 'LANG08 Annotation'
  $window=$reopened.Window
  if($null -eq $reopened.Editor){throw 'LANG-08 restart returned no editor'}
  Readback-Selected -Process $proc -Expected $text
  Close-Note -Process $proc;$proc=$null
  if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $beforeHash -or
     (Read-AnnotationState -Path $db) -cne $dbState){
    throw 'LANG-08 restart modified Markdown or saved annotation SQLite'
  }
  Assert-Upstream
  Write-Host 'AKULA_LANG08_INSTALLED_RESTART_PASS genuine_nsis=1 reopened=1 sqlite_unchanged=1 markdown_sha256=1'

  # LANG-08b starts after the independent LANG-08a restart witness.
  # Everything below runs on a genuine, unsigned, installed Akula binary.
  $backup=Join-Path $env:RUNNER_TEMP 'lang08b-disposable.annotations.json'
  $archive=Join-Path $env:RUNNER_TEMP 'lang08b-original-sidecar.sqlite'
  if((Test-Path -LiteralPath $backup) -or (Test-Path -LiteralPath $archive)){
    throw 'LANG-08b refuses stale disposable backup/archive fixture'
  }
  $original=Get-AnnotationSnapshot -Path $db
  if($original.record_count -ne 1 -or $original.status -cne 'current' -or
     $original.revision -lt 1){throw 'LANG-08b expected one genuine current highlight'}
  $proc=Open-Note -Note $note -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08 Annotation'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08 Annotation'
  $window=$ready.Window
  Readback-Selected -Process $proc -Expected $text
  $actions=Wait-Visible -Root $window -Names $labels.actions -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $actions -Window $window -Process $proc
  $export=Wait-Visible -Root $window -Names $labels.backupExport -Type ([Windows.Automation.ControlType]::MenuItem)
  $appWindowHandle=[IntPtr]$proc.MainWindowHandle
  Click-Visible -Target $export -Window $window -Process $proc
  Choose-FileThroughNativeDialog -Process $proc -Path $backup -AppWindowHandle $appWindowHandle
  $backupReady=$false
  for($i=0;$i -lt 90;$i++){
    Start-Sleep -Milliseconds 200
    if(Test-Path -LiteralPath $backup -PathType Leaf){
      if((Get-Item -LiteralPath $backup).Length -gt 40){$backupReady=$true;break}
    }
  }
  if(!$backupReady){throw 'LANG-08b real Save dialog did not create synthetic backup'}
  $backupJson=[IO.File]::ReadAllText($backup,[Text.Encoding]::UTF8)|ConvertFrom-Json
  # Rust serde serializes TextAnchor.selected_digest as an array of 32
  # byte values. Compare stable lowercase hex, never PS array identity.
  $backupDigestValues=@($backupJson.records[0].anchor.selected_digest)
  if($backupDigestValues.Count -ne 32){throw 'LANG-08b invalid backup digest length'}
  $backupDigestBytes=New-Object byte[] 32
  for($i=0;$i -lt 32;$i++){
    # Windows PowerShell ConvertFrom-Json can represent JSON integers as
    # Int64 rather than Int32. Accept either native integer representation,
    # but refuse booleans, floating point values and out-of-range bytes.
    $value=$backupDigestValues[$i]
    if($value -is [bool] -or
       !($value -is [int] -or $value -is [long]) -or
       $value -lt 0 -or $value -gt 255){
      throw 'LANG-08b invalid backup digest byte'
    }
    $backupDigestBytes[$i]=[byte]$value
  }
  $backupDigest=[BitConverter]::ToString($backupDigestBytes).Replace('-','').ToLowerInvariant()
  if($backupJson.format -cne 'akula-text-annotations' -or $backupJson.version -ne 1 -or
     @($backupJson.records).Count -ne 1 -or
     $backupJson.records[0].status -cne 'current' -or
     $backupJson.records[0].anchor.annotation_id -cne $original.annotation_id -or
     $backupDigest -cne $original.selected_digest -or
     ([IO.File]::ReadAllText($backup)).Contains('alpha eat omega')){
    throw 'LANG-08b saved backup has invalid metadata scope or leaked Markdown'
  }
  $backupHash=(Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash
  Assert-SyntheticUnchanged -Markdown $note -ExpectedHash $beforeHash -Backup $backup -BackupHash $backupHash
  $notice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $notice -Window $window -Process $proc
  Start-Sleep -Milliseconds 400
  Close-Note -Process $proc;$proc=$null
  Write-Host 'AKULA_LANG08B_INSTALLED_EXPORT_PASS native_save_picker=1 document_scoped=1 json=1 markdown_sha256=1'

  # The *disposable* sidecar is archived only with Akula closed, so import
  # cannot silently merge into existing metadata or overwrite the old record.
  # Checkpoint, close and then move the actual SQLite file; refuse stale WAL.
  $checkpoint=@'
import sqlite3
import sys
with sqlite3.connect(sys.argv[1]) as db:
    result=db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
if result is None or result[0]!=0:
    raise ValueError("SQLite WAL checkpoint busy")
'@
  $null=($checkpoint | & python - $db)
  if($LASTEXITCODE -ne 0){throw 'LANG-08b failed synthetic SQLite checkpoint'}
  foreach($side in @($db+'-wal',$db+'-shm')){
    if(Test-Path -LiteralPath $side){
      throw 'LANG-08b refuses an unclosed SQLite WAL or SHM file'
    }
  }
  if((Read-AnnotationState -Path $db) -cne $dbState){
    throw 'LANG-08b changed sidecar before archiving'
  }
  Move-Item -LiteralPath $db -Destination $archive -ErrorAction Stop
  if(Test-Path -LiteralPath $db){throw 'LANG-08b expected a cold sidecar after archive'}

  $proc=Open-Note -Note $note -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08 Annotation'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08 Annotation'
  $window=$ready.Window
  Readback-Selected -Process $proc -Expected $text
  if(Test-Path -LiteralPath $db){
    $cold=Get-AnnotationSnapshot -Path $db
    if($cold.record_count -ne 0 -or $cold.revision -ne 0){
      throw 'LANG-08b fresh sidecar was not empty before import'
    }
  }
  $actions=Wait-Visible -Root $window -Names $labels.actions -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $actions -Window $window -Process $proc
  $import=Wait-Visible -Root $window -Names $labels.backupImport -Type ([Windows.Automation.ControlType]::MenuItem)
  $appWindowHandle=[IntPtr]$proc.MainWindowHandle
  Click-Visible -Target $import -Window $window -Process $proc
  Choose-FileThroughNativeDialog -Process $proc -Path $backup -AppWindowHandle $appWindowHandle
  $held=$null
  for($i=0;$i -lt 90;$i++){
    Start-Sleep -Milliseconds 200
    if(Test-Path -LiteralPath $db -PathType Leaf){
      $held=Get-AnnotationSnapshot -Path $db
      if($held.record_count -eq 1 -and $held.status -ceq 'needs_rebind'){break}
    }
  }
  if($null -eq $held -or $held.record_count -ne 1 -or
     $held.status -cne 'needs_rebind' -or $held.revision -lt 1 -or
     $held.annotation_id -cne $original.annotation_id -or
     $held.selected_digest -cne $original.selected_digest -or
     $held.color -cne $original.color){
    throw 'LANG-08b installed import failed to restore matching annotation on HOLD'
  }
  Assert-SyntheticUnchanged -Markdown $note -ExpectedHash $beforeHash -Backup $backup -BackupHash $backupHash
  Write-Host 'AKULA_LANG08B_INSTALLED_IMPORT_HOLD_PASS native_open_picker=1 needs_rebind=1 no_auto_attach=1'
  $notice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $notice -Window $window -Process $proc
  Start-Sleep -Milliseconds 400

  # User explicitly selects the same text with physical Ctrl+A/C, right-clicks,
  # then invokes rebind through the actual installed context menu. No Core API
  # injection and no direct SQLite mutation is used for import or rebind.
  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  Readback-Selected -Process $proc -Expected $text
  Click-Visible -Target $body -Window $window -Process $proc -Right
  $rebind=Wait-Visible -Root $window -Names $labels.rebind -Type ([Windows.Automation.ControlType]::MenuItem)
  Click-Visible -Target $rebind -Window $window -Process $proc
  $restored=$null
  for($i=0;$i -lt 90;$i++){
    Start-Sleep -Milliseconds 200
    $restored=Get-AnnotationSnapshot -Path $db
    if($restored.status -ceq 'current' -and $restored.revision -gt $held.revision){break}
  }
  if($null -eq $restored -or $restored.status -cne 'current' -or
     $restored.record_count -ne 1 -or $restored.revision -le $held.revision -or
     $restored.annotation_id -cne $original.annotation_id -or
     $restored.selected_digest -cne $original.selected_digest -or
     $restored.color -cne $original.color){
    throw 'LANG-08b selected installed rebind did not convert HOLD to Current'
  }
  Assert-SyntheticUnchanged -Markdown $note -ExpectedHash $beforeHash -Backup $backup -BackupHash $backupHash
  Write-Host 'AKULA_LANG08B_INSTALLED_REBIND_PASS real_os_selection=1 explicit_menu=1 sqlite_cas=1 markdown_sha256=1'
  $notice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $notice -Window $window -Process $proc
  Close-Note -Process $proc;$proc=$null
  $restoredState=Read-AnnotationState -Path $db
  $proc=Open-Note -Note $note -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08 Annotation'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08 Annotation'
  Readback-Selected -Process $proc -Expected $text
  Close-Note -Process $proc;$proc=$null
  if((Read-AnnotationState -Path $db) -cne $restoredState){
    throw 'LANG-08b installed restart changed restored SQLite annotation'
  }
  Assert-SyntheticUnchanged -Markdown $note -ExpectedHash $beforeHash -Backup $backup -BackupHash $backupHash
  Assert-Upstream
  Write-Host 'AKULA_LANG08B_INSTALLED_RESTART_PASS reopened=1 restored_sqlite_stable=1 source_untouched=1'

  # LANG-08c: a real installed UIA/OS-backed named collections lifecycle.
  # The prior LANG-08b suite has already verified this unsigned NSIS,
  # annotation sidecar and original Markdown; it is not modified here.
  $studyNote=Join-Path $vault 'LANG08C Collections.md'
  $studyText='| Q | A |'+[char]10+'| --- | --- |'+[char]10+
    '| oxygen | gas |'+[char]10+'| sodium | metal |'+[char]10
  [IO.File]::WriteAllText($studyNote,$studyText,[Text.UTF8Encoding]::new($false))
  $studyHash=(Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash
  $studyDb=Join-Path $data 'language-study.sqlite'
  if(Test-Path -LiteralPath $studyDb){
    $cold=Get-StudySnapshot -Path $studyDb
    if($cold.revision -ne 0 -or $cold.count -ne 0 -or
       $cold.table_count -ne 0 -or $cold.anchor_count -ne 0){
      throw 'LANG-08c expected an empty synthetic study registry'
    }
  }
  $proc=Open-Note -Note $studyNote -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08C Collections'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08C Collections'
  $window=$ready.Window
  Readback-Selected -Process $proc -Expected $studyText

  Open-StudyCollectionMenu -Window $window -Process $proc
  Study-Choose -Window $window -Process $proc -Names $labels.studyNew
  [void](Wait-Visible -Root $window -Names $labels.studyCreate -Type ([Windows.Automation.ControlType]::Button))
  Type-StudyName -Process $proc -Name 'LANG08C-Biology'
  $create=Wait-Visible -Root $window -Names $labels.studyCreate -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $create -Window $window -Process $proc
  $created=Require-StudySnapshot -Path $studyDb -Count 1 -Name 'LANG08C-Biology'
  if($created.revision -lt 1 -or [string]::IsNullOrWhiteSpace($created.id) -or
     $created.members -ne 1 -or $created.table_count -ne 1 -or
     $created.anchor_count -ne 1){
    throw 'LANG-08c installed collection creation has wrong SQLite identity/bindings'
  }
  if((Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c native collection creation modified source Markdown'
  }
  Write-Host 'AKULA_LANG08C_INSTALLED_CREATE_PASS os_table_menu=1 native_name_dialog=1 sqlite_member=1 source_unchanged=1'
  Close-StudyNotice -Window $window -Process $proc

  # LANG-08c2a: real installed refusal for case-insensitive duplicate and
  # empty names. Establish one valid collection via the GUI above, then prove
  # each failed create leaves the real SQLite payload/revision and Markdown
  # unchanged. No test-side data mutation or bypass of the native app.
  $negativeBefore=Read-StudyStateHash -Path $studyDb
  foreach($case in @('duplicate','empty','too-long','bidi')){
    Open-StudyCollectionMenu -Window $window -Process $proc
    Study-Choose -Window $window -Process $proc -Names $labels.studyNew
    $create=Wait-Visible -Root $window -Names $labels.studyCreate -Type ([Windows.Automation.ControlType]::Button)
    if($case -ceq 'duplicate'){
      Type-StudyName -Process $proc -Name 'LANG08C-BIOLOGY'
    } elseif($case -ceq 'too-long' -or $case -ceq 'bidi'){
      Type-StudyInvalidName -Process $proc -Case $case
    }
    $create=Wait-Visible -Root $window -Names $labels.studyCreate -Type ([Windows.Automation.ControlType]::Button)
    Click-Visible -Target $create -Window $window -Process $proc
    Start-Sleep -Milliseconds 200
    # A refusal must replace the create modal with an error notice, rather
    # than merely leave the pending create dialog open or silently succeed.
    if($null -ne (Find-Visible -Root $window -Names $labels.studyCreate -Type ([Windows.Automation.ControlType]::Button))){
      throw "LANG-08c2 $case did not leave the native create dialog"
    }
    $notice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
    $snapshot=Require-StudySnapshot -Path $studyDb -Count 1 -Name 'LANG08C-Biology'
    if($snapshot.id -cne $created.id -or $snapshot.revision -ne $created.revision -or
       $snapshot.members -ne $created.members -or
       $snapshot.table_count -ne $created.table_count -or
       $snapshot.anchor_count -ne $created.anchor_count -or
       (Read-StudyStateHash -Path $studyDb) -cne $negativeBefore -or
       (Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
      throw "LANG-08c2 $case refusal changed real SQLite identity/payload or Markdown"
    }
    if($case -ceq 'duplicate'){
      Write-Host 'AKULA_LANG08C2_INSTALLED_DUPLICATE_PASS native_refusal=1 case_insensitive=1 sqlite_unchanged=1 markdown_sha256=1'
    } elseif($case -ceq 'empty') {
      Write-Host 'AKULA_LANG08C2_INSTALLED_EMPTY_PASS native_refusal=1 empty_name=1 sqlite_unchanged=1 markdown_sha256=1'
    } elseif($case -ceq 'too-long') {
      Write-Host 'AKULA_LANG08C2B_INSTALLED_OVERLONG_PASS native_refusal=1 length_gt_64=1 sqlite_unchanged=1 markdown_sha256=1'
    } else {
      Write-Host 'AKULA_LANG08C2B_INSTALLED_BIDI_PASS native_refusal=1 bidi_control=1 sqlite_unchanged=1 markdown_sha256=1'
    }
    Click-Visible -Target $notice -Window $window -Process $proc
    Start-Sleep -Milliseconds 400
  }
  Close-Note -Process $proc;$proc=$null

  $proc=Open-Note -Note $studyNote -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08C Collections'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08C Collections'
  $window=$ready.Window
  Readback-Selected -Process $proc -Expected $studyText
  Open-StudyCollectionMenu -Window $window -Process $proc
  Study-Choose -Window $window -Process $proc -Names $labels.studyManage
  Study-Choose -Window $window -Process $proc -Names @('Rename: LANG08C-Biology')
  [void](Wait-Visible -Root $window -Names $labels.studyRenameSave -Type ([Windows.Automation.ControlType]::Button))
  Type-StudyName -Process $proc -Name 'LANG08C-Botany'
  $save=Wait-Visible -Root $window -Names $labels.studyRenameSave -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $save -Window $window -Process $proc
  $renamed=Require-StudySnapshot -Path $studyDb -Count 1 -Name 'LANG08C-Botany'
  if($renamed.revision -le $created.revision -or $renamed.id -cne $created.id -or
     $renamed.members -ne $created.members -or
     $renamed.table_count -ne $created.table_count -or
     $renamed.anchor_count -ne $created.anchor_count){
    throw 'LANG-08c installed rename changed collection identity or table mappings'
  }
  Write-Host 'AKULA_LANG08C_INSTALLED_RENAME_PASS os_dialog=1 stable_id=1 table_mappings=1'
  Close-StudyNotice -Window $window -Process $proc

  Open-StudyCollectionMenu -Window $window -Process $proc
  Study-Choose -Window $window -Process $proc -Names $labels.studyManage
  Study-Choose -Window $window -Process $proc -Names @('Delete: LANG08C-Botany')
  $cancel=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $cancel -Window $window -Process $proc
  $afterCancel=Get-StudySnapshot -Path $studyDb
  if($afterCancel.revision -ne $renamed.revision -or
     $afterCancel.id -cne $created.id -or $afterCancel.name -cne $renamed.name){
    throw 'LANG-08c cancelled installed delete modified SQLite revision or collection'
  }
  Write-Host 'AKULA_LANG08C_INSTALLED_CANCEL_PASS native_confirmation=1 sqlite_unchanged=1'
  Start-Sleep -Milliseconds 400

  Open-StudyCollectionMenu -Window $window -Process $proc
  Study-Choose -Window $window -Process $proc -Names $labels.studyManage
  Study-Choose -Window $window -Process $proc -Names @('Delete: LANG08C-Botany')
  $confirm=Wait-Visible -Root $window -Names $labels.studyDeleteConfirm -Type ([Windows.Automation.ControlType]::Button)
  Click-Visible -Target $confirm -Window $window -Process $proc
  $deleted=Require-StudySnapshot -Path $studyDb -Count 0 -Name $null
  if($deleted.revision -le $renamed.revision -or
     $deleted.table_count -ne $created.table_count -or
     $deleted.anchor_count -ne $created.anchor_count){
    throw 'LANG-08c installed delete damaged table/anchor records'
  }
  if((Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c installed delete changed source Markdown'
  }
  Write-Host 'AKULA_LANG08C_INSTALLED_DELETE_PASS native_confirmation=1 collection_gone=1 tables_preserved=1 markdown_sha256=1'
  Close-StudyNotice -Window $window -Process $proc
  Close-Note -Process $proc;$proc=$null

  $studyState=Read-StudyStateHash -Path $studyDb

  # The production default autosave debounce is 1000 ms; genuine OS/UIA
  # clipboard readback and table-menu traversal take longer than that.
  # Hold a REAL unsaved edit during this negative witness using Akula's
  # existing saveDebounceMs setting in this disposable RUNNER_TEMP profile.
  # No source file, production config, SQLite or user-owned vault is changed.
  $settingsPath=Join-Path $data 'settings.json'
  if(!(Test-Path -LiteralPath $settingsPath -PathType Leaf)){
    throw 'LANG-08c2b5 installed synthetic app settings missing'
  }
  $settings=[IO.File]::ReadAllText($settingsPath,[Text.Encoding]::UTF8) | ConvertFrom-Json
  if($null -eq $settings.editor -or $null -eq $settings.editor.saveDebounceMs){
    throw 'LANG-08c2b5 invalid installed editor settings schema'
  }
  $settings.editor.saveDebounceMs=120000
  $settingsJson=$settings | ConvertTo-Json -Depth 100
  [IO.File]::WriteAllText($settingsPath,$settingsJson,[Text.UTF8Encoding]::new($false))
  $readbackSettings=[IO.File]::ReadAllText($settingsPath,[Text.Encoding]::UTF8) | ConvertFrom-Json
  if([int]$readbackSettings.editor.saveDebounceMs -ne 120000){
    throw 'LANG-08c2b5 synthetic autosave window was not configured'
  }
  Write-Host 'AKULA_LANG08C2B5_SYNTHETIC_DEBOUNCE_CONFIG_PASS runner_temp_only=1 debounce_ms=120000'
  $proc=Open-Note -Note $studyNote -Vault $vault -Data $data
  Wait-Window -Process $proc -Needle 'LANG08C Collections'
  Focus-Akula -Process $proc
  $ready=Wait-EditorBody -Process $proc -Needle 'LANG08C Collections'
  $window=$ready.Window
  # If Akula rejects/overwrites a test-only JSON setting, the negative
  # fixture is invalid: fail before editing rather than silently testing
  # a note that will autosave during OS/UIA traversal.
  $runtimeSettings=[IO.File]::ReadAllText($settingsPath,[Text.Encoding]::UTF8) | ConvertFrom-Json
  if((Test-Path -LiteralPath ($settingsPath+'.unreadable')) -or
     [int]$runtimeSettings.editor.saveDebounceMs -ne 120000){
    throw 'LANG-08c2b5 installed settings did not retain disposable unsaved window'
  }
  Readback-Selected -Process $proc -Expected $studyText
  if((Read-StudyStateHash -Path $studyDb) -cne $studyState -or
     (Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c installed restart changed study SQLite or Markdown'
  }
  Assert-Upstream
  Write-Host 'AKULA_LANG08C_INSTALLED_RESTART_PASS reopened=1 study_sqlite_stable=1 source_untouched=1'

  # LANG-08c2b5 reuses the already verified foreground OS/UIA editor
  # from the restart test above. The test-only long debounce prevents normal
  # autosave from racing several seconds of OS clipboard verification.
  # Real Ctrl+A/C selected the whole note; Ctrl+V must change the live
  # table before the installed Add-to-study refusal is tested.
  $dirtyText=$studyText.Replace('| oxygen | gas |','| helium | gas |')
  if($dirtyText -ceq $studyText -or $dirtyText -notmatch '\| helium \| gas \|'){
    throw 'LANG-08c2b5 invalid synthetic edited table fixture'
  }
  $hwnd=[IntPtr]$proc.MainWindowHandle
  [Windows.Forms.Clipboard]::SetText($dirtyText)
  if([Windows.Forms.Clipboard]::GetText() -cne $dirtyText){
    throw 'LANG-08c2b5 synthetic editor clipboard mismatch'
  }
  # Readback-Selected left the real editor selected via OS Ctrl+A/C.
  # Physical Ctrl+V enters a valid changed table with the editor focused.
  [AkulaLang08DialogKeys]::Chord($hwnd,$proc.Id,[uint16]0x56)
  # Winit dispatches ClipboardPaste as an editor event; replacing clipboard
  # immediately with Readback-Selected's poison can race the paste read.
  # Let the app consume the synthetic clipboard before the readback changes it.
  Start-Sleep -Milliseconds 750
  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  Write-Host 'LANG08C2B5_PASTE_DIAGNOSTIC settled_before_clipboard_poison=1'
  Readback-Selected -Process $proc -Expected $dirtyText
  if((Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c2b5 cannot establish unsaved note; fixture already persisted'
  }
  if((Read-StudyStateHash -Path $studyDb) -cne $studyState){
    throw 'LANG-08c2b5 synthetic study registry changed before refusal'
  }
  $add=Open-StudyTableMenu -Window $window -Process $proc
  Click-Visible -Target $add -Window $window -Process $proc
  $notice=Wait-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button)
  # An unsaved note must not enter the question-column picker.
  $middot=[char]0x00b7
  if($null -ne (Find-Visible -Root $window -Names @("1 $middot Q") -Type ([Windows.Automation.ControlType]::MenuItem))){
    throw 'LANG-08c2b5 dirty note reached study column mapping'
  }
  if((Read-StudyStateHash -Path $studyDb) -cne $studyState -or
     (Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c2b5 installed dirty-note refusal changed SQLite or saved Markdown'
  }
  Write-Host 'AKULA_LANG08C2B_INSTALLED_DIRTY_NOTE_PASS os_editor_paste=1 native_refusal=1 sqlite_unchanged=1 markdown_sha256=1'
  Click-Visible -Target $notice -Window $window -Process $proc
  Start-Sleep -Milliseconds 400

  # Undo the ONE verified synthetic paste through real OS Ctrl+Z.
  # After a table-menu refusal, Ctrl+A targets a selected table cell rather
  # than the document; trying to paste the whole note overwrites that cell.
  # The native table_key Undo path explicitly leaves the table session, then
  # the editor undo stack restores the entire original Markdown in memory.
  # This is not a filesystem fixture rewrite or an injected app action.
  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  [AkulaLang08DialogKeys]::Chord($hwnd,$proc.Id,[uint16]0x5A)
  Start-Sleep -Milliseconds 600
  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  Readback-Selected -Process $proc -Expected $studyText
  Close-Note -Process $proc;$proc=$null
  if((Read-StudyStateHash -Path $studyDb) -cne $studyState -or
     (Get-FileHash -LiteralPath $studyNote -Algorithm SHA256).Hash -cne $studyHash){
    throw 'LANG-08c2b5 restoring fixture changed SQLite or Markdown on installed exit'
  }
  Assert-Upstream

} finally {
  if($null -ne $proc){
    $proc.Refresh()
    if(!$proc.HasExited){$proc.Kill();$proc.WaitForExit()}
    $proc.Dispose()
  }
  if($installed){
    $uninstaller=Join-Path $install 'uninstall.exe'
    if(!(Test-Path -LiteralPath $uninstaller -PathType Leaf)){
      throw 'LANG-08 genuine NSIS uninstaller missing'
    }
    $remove=Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
    if($remove.ExitCode -ne 0 -or (Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $key)){
      throw 'LANG-08 NSIS uninstall failed or retained installed identity'
    }
    Assert-Upstream
    if($null -ne $beforeHash -and
       (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $beforeHash){
      throw 'LANG-08 uninstall changed synthetic Markdown'
    }
    Write-Host 'AKULA_LANG08_INSTALLED_UNINSTALL_PASS clean=1 upstream_untouched=1 data_preserved=1'
  }
}
