# VIDEO-TOOLS-01c: read-only OS-derived Program Files x64 guard.
# Non-shipping lab helper. No executable trust/launch permission or registry writes.
function Get-AkulaVideoToolsOsProgramFiles {
  if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
    throw 'VIDEO_TOOLS_01C_HOLD: 64-bit Windows process is required'
  }
  if ([Environment]::OSVersion.Version.Major -lt 10) {
    throw 'VIDEO_TOOLS_01C_HOLD: video tools require Windows 10+'
  }
  if (-not ('AkulaVideoToolsKnownFolder' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaVideoToolsKnownFolder {
  [DllImport("shell32.dll", ExactSpelling=true)]
  private static extern int SHGetKnownFolderPath(ref Guid id, uint flags, IntPtr token, out IntPtr result);
  public static string ProgramFilesX64() {
    // FOLDERID_ProgramFilesX64, from the Windows SDK knownfolderid.h.
    var id = new Guid("6D809377-6AF0-444B-8957-A3773F02200E");
    IntPtr ptr = IntPtr.Zero;
    int result = SHGetKnownFolderPath(ref id, 0, IntPtr.Zero, out ptr);
    if (result != 0 || ptr == IntPtr.Zero)
      throw new InvalidOperationException("VIDEO_TOOLS_01C_HOLD: OS known folder API failed");
    try {
      var value = Marshal.PtrToStringUni(ptr);
      if (String.IsNullOrWhiteSpace(value))
        throw new InvalidOperationException("VIDEO_TOOLS_01C_HOLD: OS known folder is empty");
      return value;
    } finally {
      Marshal.FreeCoTaskMem(ptr);
    }
  }
}
'@
  }
  $osKnown = [AkulaVideoToolsKnownFolder]::ProgramFilesX64()
  if (![IO.Path]::IsPathFullyQualified($osKnown)) {
    throw 'VIDEO_TOOLS_01C_HOLD: OS known folder is not absolute'
  }
  $root = [IO.Path]::GetFullPath($osKnown).TrimEnd([IO.Path]::DirectorySeparatorChar)
  if (!(Test-Path -LiteralPath $root -PathType Container)) {
    throw 'VIDEO_TOOLS_01C_HOLD: OS program directory missing'
  }
  Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
  return $root
}

function Assert-AkulaVideoToolsNoReparseParents {
  param(
    [Parameter(Mandatory=$true)][string]$Path,
    [switch]$RequireExists
  )
  if (![IO.Path]::IsPathFullyQualified($Path)) {
    throw 'VIDEO_TOOLS_01C_HOLD: relative path refused'
  }
  $full = [IO.Path]::GetFullPath($Path)
  $current = $full
  $depth = 0
  while ($null -ne $current -and $current.Length -gt 0) {
    $depth++
    if ($depth -gt 32) {
      throw 'VIDEO_TOOLS_01C_HOLD: unexpected path depth'
    }
    $info = Get-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
    if ($null -eq $info) {
      if ($RequireExists) {
        throw 'VIDEO_TOOLS_01C_HOLD: protected directory does not exist'
      }
    } else {
      if (($info.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
          !$info.PSIsContainer) {
        throw 'VIDEO_TOOLS_01C_HOLD: reparse, link or non-directory component'
      }
    }
    $parent = [IO.Directory]::GetParent($current)
    if ($null -eq $parent) { break }
    $current = $parent.FullName
  }
}

function Assert-AkulaVideoToolsLabRoot {
  param(
    [Parameter(Mandatory=$true)][string]$Path,
    [Parameter(Mandatory=$true)][string]$LabId,
    [switch]$RequireExists
  )
  if ($LabId -cnotmatch '^[a-f0-9]{32}$') {
    throw 'VIDEO_TOOLS_01C_HOLD: invalid disposable lab identity'
  }
  $osRoot = Get-AkulaVideoToolsOsProgramFiles
  $expected = Join-Path $osRoot ('AkulaVideoToolsLab-' + $LabId)
  if (![IO.Path]::IsPathFullyQualified($Path) -or
      ![IO.Path]::GetFullPath($Path).Equals(
        [IO.Path]::GetFullPath($expected), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'VIDEO_TOOLS_01C_HOLD: path outside exact OS-known product root'
  }
  Assert-AkulaVideoToolsNoReparseParents -Path $Path -RequireExists:$RequireExists
  if ($RequireExists -and !(Test-Path -LiteralPath $Path -PathType Container)) {
    throw 'VIDEO_TOOLS_01C_HOLD: installed root not a real directory'
  }
}
