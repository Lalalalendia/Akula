# LANG-08a: a genuine NSIS-installed Akula, Windows OS pointer and UIA.
# Scope: cold-vault manual HOLD refusal, real highlight/color, note-actions
# visibility, restart and source-hash preservation. NOT full backup import E2E.
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
    throw 'LANG-08 installed editor failed exact OS Ctrl+A/C clipboard readback'
  }
}
# SQLite WAL may checkpoint without changing records: compare snapshot instead
# of physical database bytes, while still requiring a genuine SQLite file.
function Read-AnnotationState {
  param([string]$Path)
  $py='import hashlib,pathlib,sqlite3,sys; con=sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+"?mode=ro",uri=True); row=con.execute("SELECT revision,payload FROM text_annotation_state WHERE id=1").fetchone(); assert row is not None and row[0]>=0; print(str(row[0])+":"+hashlib.sha256(row[1]).hexdigest()); con.close()'
  $result=(& python -c $py $Path | Out-String).Trim()
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
  $dismissed=$false
  for($i=0;$i -lt 45;$i++){
    if($null -eq (Find-Visible -Root $window -Names $labels.noticeClose -Type ([Windows.Automation.ControlType]::Button))){
      $dismissed=$true;break
    }
    Start-Sleep -Milliseconds 150
  }
  if(!$dismissed){throw 'LANG-08 denial notice was not dismissed through installed UI'}
  Write-Host 'AKULA_LANG08_INSTALLED_REBIND_DENY_PASS genuine_nsis=1 os_mouse=1 sidecar_created=0 source_unchanged=1 modal_closed=1'

  Focus-Akula -Process $proc
  $body=Select-EditorBody -Root $window
  Readback-Selected -Process $proc -Expected $text
  Click-Visible -Target $body -Window $window -Process $proc -Right
  $highlight=Wait-Visible -Root $window -Names $labels.highlight -Type ([Windows.Automation.ControlType]::MenuItem)
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
  $window=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
  [void](Select-EditorBody -Root $window)
  Readback-Selected -Process $proc -Expected $text
  Close-Note -Process $proc;$proc=$null
  if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $beforeHash -or
     (Read-AnnotationState -Path $db) -cne $dbState){
    throw 'LANG-08 restart modified Markdown or saved annotation SQLite'
  }
  Assert-Upstream
  Write-Host 'AKULA_LANG08_INSTALLED_RESTART_PASS genuine_nsis=1 reopened=1 sqlite_unchanged=1 markdown_sha256=1'
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
