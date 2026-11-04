# Installed Windows U02: create using Ctrl+O/Shift+Enter, edit, save,
# restart, search for saved content, reopen and compare exact note bytes.
param(
  [Parameter(Mandatory=$true)][string]$ExePath,
  [Parameter(Mandatory=$true)][string]$VaultPath,
  [Parameter(Mandatory=$true)][string]$DataDir
)
$ErrorActionPreference='Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
  $PSVersionTable.PSEdition -ne 'Desktop' -or
  [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
  throw 'Installed U02 is restricted to GitHub-hosted Windows PowerShell STA'
}
$exe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($exe) -or
  !(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Unexpected Akula executable' }
$runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
$vault = [IO.Path]::GetFullPath($VaultPath)
if (!$vault.StartsWith($runner,[StringComparison]::OrdinalIgnoreCase) -or
  !(Test-Path -LiteralPath $vault -PathType Container) -or
  ((Get-Item -LiteralPath $vault).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
  throw 'U02 requires a normal disposable vault in RUNNER_TEMP'
}
if ([IO.Path]::GetFullPath($DataDir) -ne
    [IO.Path]::GetFullPath((Join-Path $env:APPDATA 'io.github.lalalendia.akula'))) {
  throw 'U02 refuses unknown app data'
}
$first = Join-Path $vault 'First note.md'
$title = 'AkulaU02Created'
$created = Join-Path $vault ($title + '.md')
if (!(Test-Path -LiteralPath $first -PathType Leaf) -or
    (Test-Path -LiteralPath $created)) { throw 'U02 fixture precondition failed' }
$firstHash = (Get-FileHash -LiteralPath $first -Algorithm SHA256).Hash
$marker = 'u02journey' + [guid]::NewGuid().ToString('N')
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
$process = $null

function U02-Focus {
  param([Diagnostics.Process]$P)
  $P.Refresh()
  if ($P.HasExited -or $P.MainWindowHandle -eq [IntPtr]::Zero -or
      $P.MainWindowTitle -notlike '*Akula*') { throw 'U02 Akula window absent' }
  $h=[IntPtr]$P.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($h,9) | Out-Null
  for($i=0;$i -lt 20;$i++){
    [AkulaBetaGuiInput]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 150
    if([AkulaBetaGuiInput]::GetForegroundWindow() -eq $h){return $h}
  }
  throw 'U02 cannot focus the expected Akula window'
}
function U02-Await {
  param([Diagnostics.Process]$P,[string]$Title)
  for($i=0;$i -lt 120;$i++){
    Start-Sleep -Milliseconds 200
    $P.Refresh()
    if($P.HasExited){throw 'Akula closed during U02'}
    if($P.MainWindowHandle -ne [IntPtr]::Zero -and
       $P.MainWindowTitle.StartsWith($Title,[StringComparison]::Ordinal) -and
       $P.MainWindowTitle -like '*Akula*'){return}
  }
  throw 'U02 expected note title not shown'
}
function U02-Editor {
  param([Diagnostics.Process]$P)
  $h=U02-Focus $P
  $root=[Windows.Automation.AutomationElement]::FromHandle($h)
  if($null -eq $root){throw 'U02 has no UIA root'}
  $condition=[Windows.Automation.OrCondition]::new([Windows.Automation.Condition[]]@(
    [Windows.Automation.PropertyCondition]::new(
      [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit),
    [Windows.Automation.PropertyCondition]::new(
      [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Document)))
  for($attempt=0;$attempt -lt 35;$attempt++){
    try{
      $items=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
      $candidates=@()
      $win=$root.Current.BoundingRectangle
      for($i=0;$i -lt $items.Count;$i++){
        $el=$items.Item($i); $b=$el.Current.BoundingRectangle
        if(!$el.Current.IsEnabled -or !$el.Current.IsKeyboardFocusable -or
           $el.Current.IsOffscreen -or $b.IsEmpty -or $b.Width -lt 160 -or
           $b.Height -lt 10 -or $b.Left -lt $win.Left -or
           $b.Right -gt $win.Right -or $b.Top -lt $win.Top -or $b.Bottom -gt $win.Bottom){continue}
        $candidates += $el
      }
      if($candidates.Count -gt 0){
        $body=@($candidates | Sort-Object { $_.Current.BoundingRectangle.Top })[-1]
        $body.SetFocus(); Start-Sleep -Milliseconds 150
        if($body.Current.HasKeyboardFocus -and [AkulaBetaGuiInput]::GetForegroundWindow() -eq $h){
          return $h
        }
      }
    }catch [Windows.Automation.ElementNotAvailableException]{}
    Start-Sleep -Milliseconds 200
  }
  throw 'U02 failed to focus the installed Markdown body'
}
function U02-Launch {
  param([string]$OpenFile)
  $args=('--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$DataDir,$OpenFile)
  return (Start-Process -FilePath $exe -ArgumentList $args -PassThru)
}
function U02-Close {
  param([Diagnostics.Process]$P)
  if(!$P.CloseMainWindow() -or !$P.WaitForExit(20000)){throw 'U02 app did not close'}
}
function U02-Normal {
  param([string]$Text)
  $cr=[string][char]13; $lf=[string][char]10
  return $Text.Replace($cr+$lf,$lf).Replace($cr,$lf)
}
try{
  $process=U02-Launch $first; U02-Await $process 'First note'
  $h=U02-Focus $process
  [AkulaBetaGuiInput]::PressCtrlShortcut($h,$process.Id,[uint16]0x4F)
  Start-Sleep -Milliseconds 400
  [AkulaBetaGuiInput]::TypeAsciiKeys($h,$process.Id,$title)
  [AkulaBetaGuiInput]::SendNavigationKey($h,$process.Id,[uint16]0x0D,$true)
  U02-Await $process $title
  if(!(Test-Path -LiteralPath $created -PathType Leaf)){throw 'U02 native create did not publish file'}
  $h=U02-Editor $process
  [AkulaBetaGuiInput]::TypeAsciiKeys($h,$process.Id,$marker)
  $saved=$false
  for($i=0;$i -lt 150;$i++){
    Start-Sleep -Milliseconds 200
    $process.Refresh()
    if($process.HasExited){throw 'U02 exited before save'}
    try{if([IO.File]::ReadAllText($created).Contains($marker)){$saved=$true;break}}
    catch [IO.IOException]{}
  }
  if(!$saved){throw 'U02 did not save user-typed text through File Gate'}
  U02-Close $process; $process.Dispose();$process=$null
  $source=[IO.File]::ReadAllText($created)
  if(!$source.Contains($marker) -or
      $source.IndexOf($marker,[StringComparison]::Ordinal) -ne
      $source.LastIndexOf($marker,[StringComparison]::Ordinal)){throw 'U02 GUI marker is missing or duplicated'}
  $hash=(Get-FileHash -LiteralPath $created -Algorithm SHA256).Hash
  if((Get-FileHash -LiteralPath $first -Algorithm SHA256).Hash -ne $firstHash){
    throw 'U02 modified another note'
  }
  $process=U02-Launch $first; U02-Await $process 'First note'
  $h=U02-Focus $process
  $root=[Windows.Automation.AutomationElement]::FromHandle($h)
  [AkulaBetaGuiInput]::PressCtrlShortcut($h,$process.Id,[uint16]0x4F)
  Start-Sleep -Milliseconds 400
  [AkulaBetaGuiInput]::TypeAsciiKeys($h,$process.Id,$marker)
  $byName=[Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::NameProperty,$title)
  $found=$false
  for($i=0;$i -lt 180;$i++){
    Start-Sleep -Milliseconds 200
    $process.Refresh()
    if($process.HasExited){throw 'U02 search process exited'}
    try{
      $matches=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$byName)
      for($j=0;$j -lt $matches.Count;$j++){
        $el=$matches.Item($j)
        if(!$el.Current.IsOffscreen -and
           ($el.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem -or
            $el.Current.ControlType -eq [Windows.Automation.ControlType]::DataItem)){
          $found=$true;break
        }
      }
    }catch [Windows.Automation.ElementNotAvailableException]{}
    if($found){break}
  }
  if(!$found){throw 'U02 cannot find created note by its saved content'}
  if([AkulaBetaGuiInput]::GetForegroundWindow() -ne $h){throw 'U02 search lost window focus'}
  [AkulaBetaGuiInput]::SendNavigationKey($h,$process.Id,[uint16]0x0D,$false)
  U02-Await $process $title
  $h=U02-Editor $process
  $poison='U02_CLIPBOARD_UNCHANGED_' + [guid]::NewGuid().ToString('N')
  Set-Clipboard -Value $poison -ErrorAction Stop
  [AkulaBetaGuiInput]::PressCtrlShortcut($h,$process.Id,[uint16]0x41)
  Start-Sleep -Milliseconds 200
  [AkulaBetaGuiInput]::PressCtrlShortcut($h,$process.Id,[uint16]0x43)
  $copied=$null
  for($i=0;$i -lt 60;$i++){
    Start-Sleep -Milliseconds 150
    try{
      $value=Get-Clipboard -Raw -ErrorAction Stop
      if($null -ne $value -and $value -ne $poison){$copied=$value;break}
    }catch{}
  }
  if($null -eq $copied -or
     ![string]::Equals((U02-Normal $copied),(U02-Normal $source),[StringComparison]::Ordinal)){
    throw 'U02 installed editor did not return the exact saved note including trailing lines'
  }
  U02-Close $process;$process.Dispose();$process=$null
  if((Get-FileHash -LiteralPath $first -Algorithm SHA256).Hash -ne $firstHash -or
     (Get-FileHash -LiteralPath $created -Algorithm SHA256).Hash -ne $hash){
    throw 'U02 restart or search modified original bytes'
  }
  if(@(Get-ChildItem -LiteralPath $vault -Filter 'AkulaU02Created*.md' -File).Count -ne 1){
    throw 'U02 duplicated the created note'
  }
  Write-Host 'AKULA_WINDOWS_U02_INSTALLED_PASS create=1 save=1 restart=1 search=1 open=1 readback=1 sha256=1 duplicate=0'
}finally{
  if($null -ne $process){
    $process.Refresh()
    if(!$process.HasExited){Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue}
    $process.Dispose()
  }
}
