# Akula M3b: real Windows OS-key Undo/Redo/readback; disposable CI only.
param([string]$Binary,[string]$Out,[switch]$SelfTest)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'

function New-Fixture([string]$file,[int]$count) {
  if ($count -notin @(32,30000,100000) -or (Test-Path -LiteralPath $file)) { throw 'Unsafe fixture request' }
  $dir=Split-Path -Parent $file
  if (Test-Path -LiteralPath $dir) { throw 'Vault already exists' }
  [System.IO.Directory]::CreateDirectory($dir) | Out-Null
  $writer=[System.IO.StreamWriter]::new($file,$false,[System.Text.UTF8Encoding]::new($false))
  # Use the same LF newline convention as the Rust Markdown editor.
  $writer.NewLine=[string][char]10
  try {
    for($i=0;$i -lt $count;$i++) {
      if($i -eq 0) { $line="# Akula M3b test $count" }
      elseif($i % 997 -eq 0) { $line="## Section $i" }
      elseif($i % 17 -eq 0) { $line="- Item $i **bold** and *italic*" }
      else { $line="Paragraph $i deterministic native editor content." }
      $writer.WriteLine($line)
    }
  } finally { $writer.Dispose() }
  if(([System.IO.File]::ReadAllLines($file)).Length -ne $count) { throw 'Wrong number of lines' }
  return [pscustomobject]@{sha=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash;bytes=(Get-Item $file).Length;lines=$count}
}
if($SelfTest) {
  $dir=Join-Path ([System.IO.Path]::GetTempPath()) ('m3b-'+[guid]::NewGuid().ToString('N'))
  try {
    $test=Join-Path $dir 'vault\First note.md'
    $meta=New-Fixture $test 32
    if(!$meta.sha -or $meta.bytes -le 600) { throw 'Fixture self test failed' }
    Write-Host 'M3B_FIXTURE_SELFTEST_PASS'
  } finally { if(Test-Path $dir) { Remove-Item $dir -Force -Recurse } }
  exit 0
}
if($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or [IntPtr]::Size -ne 8) { throw 'Hosted Windows x64 only' }
if(!$Binary -or !(Test-Path $Binary) -or !$Out) { throw 'Missing binary or output' }
if((Test-Path $Out) -and (Get-ChildItem $Out -Force | Select-Object -First 1)) { throw 'Output must be empty' }
[System.IO.Directory]::CreateDirectory($Out) | Out-Null
. (Join-Path (Split-Path $PSScriptRoot -Parent) 'windows-beta-gui-input.ps1')

# Deliberately limited to Ctrl+Z and Ctrl+Y, with HWND/PID/foreground checks.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaM3bUndo {
  [StructLayout(LayoutKind.Explicit, Size=40)]
  public struct Input {
    [FieldOffset(0)] public uint Type;
    [FieldOffset(8)] public ushort VK;
    [FieldOffset(10)] public ushort Scan;
    [FieldOffset(12)] public uint Flags;
    [FieldOffset(16)] public uint Time;
    [FieldOffset(24)] public IntPtr Extra;
  }
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,[In] Input[] input,int size);
  public static void Chord(IntPtr hwnd,int expectedPid,bool redo) {
    if(hwnd==IntPtr.Zero || Marshal.SizeOf(typeof(Input))!=40)
      throw new InvalidOperationException("Unexpected x64 HWND/input layout");
    uint owner;
    if(GetWindowThreadProcessId(hwnd,out owner)==0 || owner!=(uint)expectedPid || GetForegroundWindow()!=hwnd)
      throw new InvalidOperationException("Unsafe OS keyboard target");
    const uint DOWN=1, UP=2;
    ushort vk=(ushort)(redo ? 0x59 : 0x5a);
    var keys=new Input[] {
      new Input {Type=DOWN,VK=0x11},
      new Input {Type=DOWN,VK=vk},
      new Input {Type=DOWN,VK=vk,Flags=UP},
      new Input {Type=DOWN,VK=0x11,Flags=UP}
    };
    if(SendInput((uint)keys.Length,keys,Marshal.SizeOf(typeof(Input)))!=keys.Length)
      throw new InvalidOperationException("Incomplete Undo/Redo key delivery");
  }
}
'@ -ErrorAction Stop

function Focus-Note([System.Diagnostics.Process]$p) {
  $p.Refresh()
  if($p.HasExited -or $p.MainWindowHandle -eq [IntPtr]::Zero -or
     $p.MainWindowTitle -notlike 'First note*' -or $p.MainWindowTitle -notlike '*Akula*') { throw 'Wrong note window' }
  $hwnd=[IntPtr]$p.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd,9) | Out-Null
  for($i=0;$i -lt 16;$i++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 220
    if([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
  }
  throw 'Could not focus exact note'
}
function Wait-FileState([string]$path,[scriptblock]$test) {
  $sw=[System.Diagnostics.Stopwatch]::StartNew()
  while($sw.Elapsed.TotalSeconds -lt 45) {
    try {
      $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256 -ErrorAction Stop).Hash
      if(& $test $hash) { return $hash }
    } catch [System.IO.IOException] { }
    Start-Sleep -Milliseconds 330
  }
  throw 'Expected GUI autosave state not reached'
}
function Readback-Exact([System.Diagnostics.Process]$p,[string]$path) {
  Set-Clipboard -Value ('M3B_POISON_'+[guid]::NewGuid().ToString('N')) -ErrorAction Stop
  $poison=Get-Clipboard -Raw
  Send-AkulaBetaShortcut -Process $p -Action SelectAll
  Start-Sleep -Milliseconds 400
  Send-AkulaBetaShortcut -Process $p -Action Copy
  $copied=$null
  for($i=0;$i -lt 30;$i++) {
    Start-Sleep -Milliseconds 350
    $v=Get-Clipboard -Raw -ErrorAction SilentlyContinue
    if(![string]::IsNullOrEmpty($v) -and $v -ne $poison) { $copied=$v; break }
  }
  if($null -eq $copied) { throw 'UI clipboard poison not replaced' }
  $crlf=([string][char]13)+([string][char]10)
  $lf=[string][char]10
  $expected=[System.IO.File]::ReadAllText($path).Replace($crlf,$lf).TrimEnd([char]10)
  $actual=$copied.Replace($crlf,$lf).Replace([string][char]13,$lf).TrimEnd([char]10)
  if(![string]::Equals($actual,$expected,[StringComparison]::Ordinal)) { throw 'GUI copy differs from restored note' }
  return $actual.Length
}
function Run-Case([int]$count) {
  $root=Join-Path $Out "size-$count"
  [System.IO.Directory]::CreateDirectory($root) | Out-Null
  $note=Join-Path $root 'vault\First note.md'
  $meta=New-Fixture $note $count
  $profile=Join-Path $root 'profile'
  [System.IO.Directory]::CreateDirectory($profile) | Out-Null
  $stamp=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  $events=New-Object 'System.Collections.Generic.List[object]'
  $samples=Join-Path $root 'samples.csv'
  $stop=Join-Path $root 'sampler.stop'
  $log=Join-Path $root 'process.log'
  $marker='AKULA_M3B_'+[guid]::NewGuid().ToString('N').ToUpperInvariant()
  $launch='--vault "'+$((Split-Path -Parent $note))+'" --data-dir "'+$profile+'" --open "'+$note+'" --focus --size 1000x650 --threads 2'
  $p=$null
  $monitor=$null
  $pass=$false
  $editedHash=$null
  $readback=0
  try {
    $p=Start-Process -FilePath $Binary -ArgumentList $launch -PassThru -RedirectStandardError $log
    for($i=0;$i -lt 320;$i++) {
      $p.Refresh()
      if($p.HasExited) { throw 'Akula exited before editor ready' }
      if($p.MainWindowTitle -like 'First note*' -and $p.MainWindowTitle -like '*Akula*') { break }
      Start-Sleep -Milliseconds 250
    }
    $null=Focus-Note $p
    $monitor=Start-Job -ArgumentList $p.Id,$samples,$stop -ScriptBlock {
      param([int]$TargetPid,[string]$Csv,[string]$StopFile)
      $w=[System.IO.StreamWriter]::new($Csv,$false,[System.Text.UTF8Encoding]::new($false))
      try {
        $w.WriteLine('unix_ms,pid,private_bytes,working_set_bytes,peak_working_set_bytes,cpu_ms,handles')
        while(!(Test-Path $StopFile)) {
          $proc=Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
          if(!$proc) { break }
          $proc.Refresh()
          $time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
          $cpu=[long][Math]::Round($proc.TotalProcessorTime.TotalMilliseconds)
          $w.WriteLine("$time,$TargetPid,$($proc.PrivateMemorySize64),$($proc.WorkingSet64),$($proc.PeakWorkingSet64),$cpu,$($proc.HandleCount)")
          $w.Flush()
          Start-Sleep -Milliseconds 250
        }
      } finally { $w.Dispose() }
    }
    Start-Sleep -Seconds 4
    $events.Add([pscustomobject]@{name='baseline';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    Send-AkulaBetaText -Process $p -Text $marker
    $events.Add([pscustomobject]@{name='typed';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $null=Wait-FileState $note {
      param($hash)
      if($hash -eq $meta.sha) { return $false }
      try {
        $body=[System.IO.File]::ReadAllText($note)
        return $body.Contains($marker) -and ([regex]::Matches($body,[regex]::Escape($marker)).Count -eq 1)
      } catch [System.IO.IOException] { return $false }
    }
    $editedHash=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
    $events.Add([pscustomobject]@{name='edit_saved';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $hwnd=Focus-Note $p
    [AkulaM3bUndo]::Chord($hwnd,$p.Id,$false)
    $events.Add([pscustomobject]@{name='undo';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $null=Wait-FileState $note {param($hash) $hash -eq $meta.sha}
    $events.Add([pscustomobject]@{name='undo_saved';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $hwnd=Focus-Note $p
    [AkulaM3bUndo]::Chord($hwnd,$p.Id,$true)
    $events.Add([pscustomobject]@{name='redo';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $null=Wait-FileState $note {param($hash) $hash -eq $editedHash}
    $events.Add([pscustomobject]@{name='redo_saved';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $hwnd=Focus-Note $p
    [AkulaM3bUndo]::Chord($hwnd,$p.Id,$false)
    $events.Add([pscustomobject]@{name='final_undo';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $null=Wait-FileState $note {param($hash) $hash -eq $meta.sha}
    $events.Add([pscustomobject]@{name='restored_saved';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    Start-Sleep -Seconds 3
    $events.Add([pscustomobject]@{name='clipboard_start';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $readback=Readback-Exact $p $note
    $events.Add([pscustomobject]@{name='readback_pass';time=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()})
    $p.Refresh()
    if(!$p.CloseMainWindow() -or !$p.WaitForExit(20000) -or $p.ExitCode -ne 0) { throw 'Non-graceful GUI exit' }
    if((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $meta.sha) { throw 'Closing Akula changed source' }
    $pass=$true
  } finally {
    if($p -and !$p.HasExited) { $p.Kill(); $null=$p.WaitForExit(15000) }
    [System.IO.File]::WriteAllText($stop,'stop')
    if($monitor) {
      $null=Wait-Job $monitor -Timeout 25
      if($monitor.State -eq 'Running') { Stop-Job $monitor }
      if($monitor.State -eq 'Failed') { throw 'Memory sampler process failed' }
      Remove-Job $monitor -Force
    }
    $events | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $root 'events.json') -Encoding UTF8
  }
  if(!$pass -or !$editedHash -or $editedHash -eq $meta.sha) { throw 'Incomplete native Undo acceptance' }
  $rows=@(Import-Csv $samples)
  $copyAt=@($events | Where-Object name -eq 'clipboard_start')[0].time
  $valid=@($rows | Where-Object {[long]$_.unix_ms -lt [long]$copyAt})
  if($valid.Count -lt 12) { throw 'Insufficient pre-clipboard samples' }
  $max=($valid | ForEach-Object {[long]$_.private_bytes} | Measure-Object -Maximum).Maximum
  $peak=($valid | ForEach-Object {[long]$_.peak_working_set_bytes} | Measure-Object -Maximum).Maximum
  $result=[pscustomobject]@{
    lines=$count;bytes=$meta.bytes;original_sha256=$meta.sha
    observations=$rows.Count;pre_clipboard=$valid.Count
    private_peak_mib=[Math]::Round([double]$max/1MB,3)
    private_restored_mib=[Math]::Round([double]$valid[-1].private_bytes/1MB,3)
    working_set_peak_mib=[Math]::Round([double]$peak/1MB,3)
    gui_readback_chars=$readback;edit=$true;undo=$true;redo=$true;final_undo=$true;gui_exact=$true;graceful=$pass
  }
  $result | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $root 'result.json') -Encoding UTF8
  Write-Host "M3B_PASS lines=$count undo=1 redo=1 final_undo=1 ui_exact=1 private_peak_mib=$($result.private_peak_mib)"
  return $result
}
$envInfo=[pscustomobject]@{
  source_sha=$env:AKULA_PROBE_SOURCE_HEAD
  exe_sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $Binary).Hash
  mode='Windows CI Release --features installed, unpackaged, synthetic fixture'
}
$envInfo | ConvertTo-Json | Set-Content (Join-Path $Out 'environment.json') -Encoding UTF8
$results=@()
foreach($size in @(30000,100000)) { $results+=Run-Case $size }
$results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Out 'results.json') -Encoding UTF8
$report=@('# Akula M3b Windows native Undo/Redo E2E diagnostic','',
  "Source: $($envInfo.source_sha); EXE SHA-256: $($envInfo.exe_sha256)",'',
  '| Lines | Edit | Undo | Redo | Final Undo | Exact GUI readback | Sampled peak Private MiB | Restored Private MiB |',
  '|---:|---|---|---|---|---|---:|---:|')
foreach($row in $results) { $report+="| $($row.lines) | PASS | PASS | PASS | PASS | PASS | $($row.private_peak_mib) | $($row.private_restored_mib) |" }
$report+='Clipboard readback is not part of the reported memory peak.'
$report+='Single diagnostic run per size, not product-grade memory history limit or NSIS validation.'
$report | Set-Content (Join-Path $Out 'summary.md') -Encoding UTF8
$report | ForEach-Object { Write-Host $_ }
