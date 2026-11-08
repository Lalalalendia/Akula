# Real native HTML preview, OS mouse and UI Automation on disposable Windows CI.
param([Parameter(Mandatory=$true)][string]$ExePath,[Parameter(Mandatory=$true)][string]$OutputDir)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'Real HTML GUI acceptance is allowed only on hosted Windows runners'
}
if ([IntPtr]::Size -ne 8 -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
  throw 'Requires x64 Windows PowerShell STA'
}
$exe = (Resolve-Path -LiteralPath $ExePath -ErrorAction Stop).Path
if ([IO.Path]::GetFileName($exe) -ne 'aquilum-native.exe') { throw 'Unexpected executable' }
$vault = Join-Path $env:RUNNER_TEMP 'html-preview-synthetic-vault'
$data = Join-Path $env:RUNNER_TEMP 'html-preview-synthetic-data'
foreach ($dir in @($vault,$data,$OutputDir)) {
  if (Test-Path -LiteralPath $dir) { throw "Nonempty test directory: $dir" }
  New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
}
$html = Join-Path $vault 'article.html'
$markdown = Join-Path $vault 'another.md'
$markup = '<!doctype html><title>Akula HTML</title><h1>AKULA_HTML_GUI_VISIBLE_813</h1>' +
  '<p>AKULA_HTML_TEXT_SECOND_813</p><ul><li>First<li>Second</ul>' +
  '<script>window.AKULA_HTML_SCRIPT_FORBIDDEN_813 = true;</script>'
[IO.File]::WriteAllText($html,$markup,(New-Object Text.UTF8Encoding($false)))
[IO.File]::WriteAllText($markdown,'# AKULA_MD_GUI_VISIBLE_813')
$h1 = (Get-FileHash -LiteralPath $html -Algorithm SHA256).Hash
$h2 = (Get-FileHash -LiteralPath $markdown -Algorithm SHA256).Hash
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class HtmlPilotMouse {
  [StructLayout(LayoutKind.Explicit,Size=40)]
  public struct Input {
    [FieldOffset(0)] public uint Type;
    [FieldOffset(20)] public uint Flags;
  }
  [StructLayout(LayoutKind.Sequential)]
  public struct Point { public int X; public int Y; }
  [DllImport("user32.dll",SetLastError=true)]
  static extern uint SendInput(uint n,[In] Input[] p,int size);
  [DllImport("user32.dll",SetLastError=true)]
  static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr p,uint n);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll",SetLastError=true)]
  static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
  public static void Click(IntPtr hwnd,int expectedPid,int x,int y) {
    uint owner;
    if (IntPtr.Size!=8 || Marshal.SizeOf(typeof(Input))!=40 || hwnd==IntPtr.Zero ||
        GetWindowThreadProcessId(hwnd,out owner)==0 || owner!=(uint)expectedPid ||
        GetForegroundWindow()!=hwnd || !SetCursorPos(x,y) ||
        GetAncestor(WindowFromPoint(new Point {X=x,Y=y}),2)!=hwnd)
      throw new InvalidOperationException("Refusing pointer outside foreground Akula");
    var keys=new Input[] {
      new Input {Type=0,Flags=0x0002},
      new Input {Type=0,Flags=0x0004}
    };
    if (SendInput(2,keys,Marshal.SizeOf(typeof(Input)))!=2)
      throw new InvalidOperationException("Native mouse click failed");
  }
}
'@ -ErrorAction Stop
function Read-UIA([IntPtr]$hwnd) {
  $root=[Windows.Automation.AutomationElement]::FromHandle($hwnd)
  if ($null -eq $root) { throw 'No Akula UIA root' }
  $nodes=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  $seen=[Collections.Generic.List[string]]::new()
  for($i=0;$i -lt $nodes.Count;$i++) {
    try {
      $node=$nodes.Item($i)
      $name=[string]$node.Current.Name
      if (![string]::IsNullOrWhiteSpace($name)) { $seen.Add($name) }
      $v=$node.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty)
      if ($null -ne $v -and $v -ne [Windows.Automation.AutomationElement]::NotSupported) {
        $value=[string]$v
        if (![string]::IsNullOrWhiteSpace($value)) { $seen.Add($value) }
      }
    } catch { }
  }
  return ($seen -join [Environment]::NewLine)
}
function Wait-UIA([Diagnostics.Process]$proc,[string]$expected,[string]$forbidden,[string]$phase) {
  $last=''
  for($n=0;$n -lt 100;$n++) {
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    if ($proc.HasExited) { throw "Akula exited in phase $phase" }
    $last=Read-UIA ([IntPtr]$proc.MainWindowHandle)
    if ($last.Contains($expected)) {
      if ($last.Contains($forbidden)) { throw "Unexpected content after $phase" }
      $last | Out-File -LiteralPath (Join-Path $OutputDir "$phase-uia.txt") -Encoding utf8
      Write-Host "AKULA_HTML_WINDOWS_STAGE_PASS stage=$phase"
      return
    }
  }
  $last | Out-File -LiteralPath (Join-Path $OutputDir "$phase-failed-uia.txt") -Encoding utf8
  throw "No accessible content after $phase"
}
function Click-File([Diagnostics.Process]$proc,[string]$name) {
  $proc.Refresh()
  if ($proc.HasExited -or $proc.MainWindowHandle -eq 0 -or $proc.MainWindowTitle -notlike '*Akula*') {
    throw 'Unexpected Akula window state'
  }
  $hwnd=[IntPtr]$proc.MainWindowHandle
  $root=[Windows.Automation.AutomationElement]::FromHandle($hwnd)
  $kind=New-Object Windows.Automation.PropertyCondition(
    [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::TreeItem)
  $item=$null
  $names=@()
  for($attempt=0;$attempt -lt 60;$attempt++){
    $list=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$kind)
    $names=@()
    for($i=0;$i -lt $list.Count;$i++){
      $entry=$list.Item($i)
      $label=[string]$entry.Current.Name
      $names+= $label
      if ($label -ceq $name) { $item=$entry; break }
    }
    if($null -ne $item){break}
    Start-Sleep -Milliseconds 250
  }
  if($null -eq $item){
    $names | Out-File -LiteralPath (Join-Path $OutputDir 'tree-items-failure.txt') -Encoding utf8
    throw "Cannot find file tree item: $name; observed $($names.Count) TreeItems"
  }
  $r=$item.Current.BoundingRectangle
  $w=$root.Current.BoundingRectangle
  if($item.Current.IsOffscreen -or $r.IsEmpty -or $r.Width -le 0 -or $r.Height -le 0){throw "Invisible file $name"}
  $x=[int][Math]::Floor($r.Left+$r.Width/2)
  $y=[int][Math]::Floor($r.Top+$r.Height/2)
  if($x -lt $w.Left -or $x -ge $w.Right -or $y -lt $w.Top -or $y -ge $w.Bottom){throw "File outside Akula window: $name"}
  [AkulaBetaGuiInput]::ShowWindow($hwnd,9)|Out-Null
  $focused=$false
  for($n=0;$n -lt 15;$n++){
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd)|Out-Null
    Start-Sleep -Milliseconds 150
    if([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd){$focused=$true;break}
  }
  if(!$focused){throw 'Failed to foreground Akula'}
  [HtmlPilotMouse]::Click($hwnd,$proc.Id,$x,$y)
}
function Save-Screenshot {
  Add-Type -AssemblyName System.Windows.Forms,System.Drawing
  $bounds=[Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bitmap=New-Object Drawing.Bitmap($bounds.Width,$bounds.Height)
  $g=[Drawing.Graphics]::FromImage($bitmap)
  try {
    $g.CopyFromScreen($bounds.Location,[Drawing.Point]::Empty,$bounds.Size)
    $bitmap.Save((Join-Path $OutputDir 'native-html.png'))
  } finally { $g.Dispose();$bitmap.Dispose() }
}
$proc=$null
try {
  $args='--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$html
  $proc=Start-Process -FilePath $exe -ArgumentList $args -PassThru
  $ready=$false
  for($i=0;$i -lt 120;$i++){
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    if($proc.HasExited){throw "Akula exited with code $($proc.ExitCode)"}
    if($proc.MainWindowHandle -ne 0 -and $proc.MainWindowTitle -like '*Akula*'){$ready=$true;break}
  }
  if(!$ready){throw 'No native Akula window'}
  Write-Host 'AKULA_HTML_WINDOWS_WINDOW_PASS'
  Wait-UIA $proc 'AKULA_HTML_GUI_VISIBLE_813' 'AKULA_HTML_SCRIPT_FORBIDDEN_813' 'html-start'
  # Markdown TreeItem is the file stem; non-Markdown attachments keep extensions.
  Click-File $proc 'another'
  Wait-UIA $proc 'AKULA_MD_GUI_VISIBLE_813' 'AKULA_HTML_GUI_VISIBLE_813' 'markdown-switch'
  Click-File $proc 'article.html'
  Wait-UIA $proc 'AKULA_HTML_TEXT_SECOND_813' 'AKULA_HTML_SCRIPT_FORBIDDEN_813' 'html-return'
  if((Get-FileHash -LiteralPath $html -Algorithm SHA256).Hash -ne $h1 -or
     (Get-FileHash -LiteralPath $markdown -Algorithm SHA256).Hash -ne $h2){
    throw 'Original file modified by preview'
  }
  Write-Host 'AKULA_HTML_WINDOWS_READONLY_PASS'
  Write-Host 'AKULA_HTML_WINDOWS_GUI_PASS native=1 html=1 markdown_switch=1 html_return=1'
} finally {
  try { Save-Screenshot } catch { Write-Host "Screenshot capture failed: $_" }
  if ($null -ne $proc) {
    $proc.Refresh()
    if (!$proc.HasExited) { Stop-Process -Id $proc.Id -Force }
  }
}
