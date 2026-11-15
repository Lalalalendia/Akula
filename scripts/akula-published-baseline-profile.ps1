# Published->candidate upgrade must start from state created by the *old*
# release. The preceding current-version installed smoke leaves a newer
# AppData profile behind, which must be retained but never fed to the old GUI.
Set-StrictMode -Version Latest

function Get-AkulaProfileManifest([string]$Root) {
  if (!(Test-Path -LiteralPath $Root -PathType Container)) {
    throw 'AKULA_PROFILE_HOLD: source profile directory is absent'
  }
  if (((Get-Item -LiteralPath $Root -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'AKULA_PROFILE_HOLD: profile root is a reparse point'
  }
  $entries = @(Get-ChildItem -LiteralPath $Root -Recurse -Force -ErrorAction Stop)
  foreach ($entry in $entries) {
    if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
      throw 'AKULA_PROFILE_HOLD: profile contains a reparse point'
    }
  }
  $files = @($entries | Where-Object { -not $_.PSIsContainer } | Sort-Object FullName)
  if ($files.Count -eq 0) { throw 'AKULA_PROFILE_HOLD: prior installed smoke profile is empty' }
  foreach ($file in $files) {
    $relative = $file.FullName.Substring($Root.TrimEnd('\').Length)
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    '{0}|{1}|{2}' -f $relative, $file.Length, $digest
  }
}

function Assert-AkulaPriorSmokeProfile([psobject]$Witness) {
  if (!(Test-Path -LiteralPath $Witness.Backup -PathType Container)) {
    throw 'AKULA_PROFILE_HOLD: preserved prior-version-smoke profile missing'
  }
  $actual = @(Get-AkulaProfileManifest $Witness.Backup)
  if ($actual.Count -ne $Witness.Manifest.Count -or
      ($actual -join "`n") -cne ($Witness.Manifest -join "`n")) {
    throw 'AKULA_PROFILE_HOLD: prior installed smoke AppData changed after isolation'
  }
}

function Reserve-AkulaPriorSmokeProfile([string]$Source, [string]$Backup) {
  $runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
  $destination = [IO.Path]::GetFullPath($Backup)
  if (!$destination.StartsWith($runner, [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFullPath($Source) -ceq $destination -or
      (Test-Path -LiteralPath $destination)) {
    throw 'AKULA_PROFILE_HOLD: backup destination is not a fresh runner-temp directory'
  }
  $manifest = @(Get-AkulaProfileManifest $Source)
  Move-Item -LiteralPath $Source -Destination $destination -ErrorAction Stop
  if (Test-Path -LiteralPath $Source) {
    throw 'AKULA_PROFILE_HOLD: newer profile still occupies default AppData'
  }
  $witness = [pscustomobject]@{ Backup = $destination; Manifest = $manifest }
  Assert-AkulaPriorSmokeProfile $witness
  return $witness
}
