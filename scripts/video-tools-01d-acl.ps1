# VIDEO-TOOLS-01d: non-shipping, read-only ACL admission for a disposable 01a root.
# This check is not a process launch grant or a TOCTOU-safe file handle.
function Assert-AkulaVideoToolsInstalledAcl {
  param(
    [Parameter(Mandatory=$true)][string]$Path,
    [Parameter(Mandatory=$true)][string]$LabId
  )
  Assert-AkulaVideoToolsLabRoot -Path $Path -LabId $LabId -RequireExists
  $names = @(
    'akula-video-mux.exe',
    'akula-yt-dlp.exe',
    'akula-quickjs.exe',
    'uninstall.exe'
  )
  $found = @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction Stop)
  if ($found.Count -ne $names.Count -or
      @($found | Where-Object { $_.Name -cnotin $names }).Count -ne 0) {
    throw 'VIDEO_TOOLS_01D_HOLD: installed package identity or file count differs'
  }
  # Only the OS and elevated installation principals may have mutation rights.
  # An admin-owned install is not equivalent to an unprivileged user's SID.
  $privileged = @(
    'S-1-5-18',      # LocalSystem
    'S-1-5-32-544',  # BUILTIN\Administrators
    'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464' # TrustedInstaller
  )
  $writeMask = [int][Security.AccessControl.FileSystemRights]::WriteData -bor
               [int][Security.AccessControl.FileSystemRights]::AppendData -bor
               [int][Security.AccessControl.FileSystemRights]::WriteAttributes -bor
               [int][Security.AccessControl.FileSystemRights]::WriteExtendedAttributes -bor
               [int][Security.AccessControl.FileSystemRights]::Delete -bor
               [int][Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor
               [int][Security.AccessControl.FileSystemRights]::ChangePermissions -bor
               [int][Security.AccessControl.FileSystemRights]::TakeOwnership
  foreach ($name in (@('') + $names)) {
    $target = if ($name -ceq '') { $Path } else { Join-Path $Path $name }
    $item = Get-Item -LiteralPath $target -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        ($name -ceq '' -and !$item.PSIsContainer) -or
        ($name -cne '' -and $item.PSIsContainer)) {
      throw 'VIDEO_TOOLS_01D_HOLD: aliased or wrong-type package object'
    }
    $acl = Get-Acl -LiteralPath $target -ErrorAction Stop
    $security = [Security.AccessControl.RawSecurityDescriptor]::new(
      $acl.GetSecurityDescriptorBinaryForm(), 0)
    if ($null -eq $security.DiscretionaryAcl) {
      throw 'VIDEO_TOOLS_01D_HOLD: null DACL grants unrestricted access'
    }
    $ownerSid = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($ownerSid -notin $privileged) {
      throw 'VIDEO_TOOLS_01D_HOLD: unsafe package owner'
    }
    foreach ($ace in $acl.GetAccessRules(
      $true, $true, [Security.Principal.SecurityIdentifier])) {
      if ($ace.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) {
        continue
      }
      # An inherit-only ACE does not apply to this object. Its effective
      # descendants are checked individually, including all three EXEs.
      if (($ace.PropagationFlags -band
           [Security.AccessControl.PropagationFlags]::InheritOnly) -ne 0) {
        continue
      }
      $sid = $ace.IdentityReference.Value
      if (([int]$ace.FileSystemRights -band $writeMask) -ne 0 -and
          $sid -notin $privileged) {
        throw ('VIDEO_TOOLS_01D_HOLD: unsafe writable ACL for ' + $name + ' SID=' + $sid)
      }
    }
  }
  Assert-AkulaVideoToolsLabRoot -Path $Path -LabId $LabId -RequireExists
}
