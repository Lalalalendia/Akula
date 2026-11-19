# VIDEO-TOOLS-01a: protected, SEPARATE three-PE NSIS identity.
# Hosted Windows PR-only; no publisher signing, release, or product install.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$MuxPath,
  [Parameter(Mandatory=$true)][string]$YtDlpPath,
  [Parameter(Mandatory=$true)][string]$QuickJsPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01A_HOLD: exact-head hosted Windows PR is required'
}
$actualHead = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualHead -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01A_HOLD: checkout does not match exact reviewed PR HEAD'
}
$principal = [Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01A_HOLD: Program Files lab must have an actual admin token'
}
if ([Environment]::OSVersion.Version.Major -lt 10) {
  throw 'VIDEO_TOOLS_01A_HOLD: video runtimes only support Windows 10+'
}
$repo = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
# VIDEO-TOOLS-01c: OS-known x64 Program Files, never ambient env:ProgramFiles.
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
$program = Get-AkulaVideoToolsOsProgramFiles
$expectedMux = Join-Path $repo 'labs/video-lite/target/release/dash.exe'
if ([IO.Path]::GetFullPath($MuxPath) -cne $expectedMux) {
  throw 'VIDEO_TOOLS_01A_HOLD: mux must be source-built from this exact checkout'
}
function Assert-RealFile {
  param([string]$Path)
  if (!(Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "VIDEO_TOOLS_01A_HOLD: missing regular file $Path"
  }
  $meta = Get-Item -LiteralPath $Path -Force
  if (($meta.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
      $meta.Length -lt 64 -or $meta.Length -gt 268435456) {
    throw 'VIDEO_TOOLS_01A_HOLD: aliased, empty, or oversized executable'
  }
  $reader = [IO.File]::OpenRead($Path)
  try {
    $buffer = [byte[]]::new(2)
    if ($reader.Read($buffer, 0, 2) -ne 2 -or
        $buffer[0] -ne 77 -or $buffer[1] -ne 90) {
      throw 'VIDEO_TOOLS_01A_HOLD: input is not a Windows PE executable'
    }
  } finally {
    $reader.Dispose()
  }
}
$allow = Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
if ($allow.schema -ne 1 -or $allow.release_admitted -ne $false -or
    $allow.installer_signing_authorized -ne $false -or
    $allow.stage_binary_distribution_authorized -ne $false -or
    $allow.independent_owner_approval_required -ne $true) {
  throw 'VIDEO_TOOLS_01A_HOLD: source/license release allowlist was promoted'
}
$inputs = [ordered]@{
  'akula-video-mux.exe' = $MuxPath
  'akula-yt-dlp.exe' = $YtDlpPath
  'akula-quickjs.exe' = $QuickJsPath
}
$digests = @{}
foreach ($name in $inputs.Keys) {
  $source = [IO.Path]::GetFullPath($inputs[$name])
  Assert-RealFile $source
  if ($name -ne 'akula-video-mux.exe') {
    if (!$source.StartsWith($runner + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
      throw 'VIDEO_TOOLS_01A_HOLD: upstream executable escaped runner temp'
    }
    $entry = @($allow.components | Where-Object { $_.name -ceq $name })
    if ($entry.Count -ne 1 -or $entry[0].kind -cne 'pinned-upstream' -or
        (Get-Item -LiteralPath $source).Length -ne $entry[0].size_bytes -or
        (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry[0].sha256) {
      throw "VIDEO_TOOLS_01A_HOLD: upstream identity/pin mismatch $name"
    }
  } else {
    $entry = @($allow.components | Where-Object { $_.name -ceq $name })
    if ($entry.Count -ne 1 -or $entry[0].kind -cne 'source-built' -or
        $entry[0].lock_path -cne 'labs/video-lite/Cargo.lock') {
      throw 'VIDEO_TOOLS_01A_HOLD: mux provenance/lock was changed'
    }
  }
  $digests[$name] = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
}
$tag = [guid]::NewGuid().ToString('N')
$work = Join-Path $runner ('video-tools-01a-' + $tag)
$root = Join-Path $program ('AkulaVideoToolsLab-' + $tag)
Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag
$stage = Join-Path $work 'stage'
$installer = Join-Path $work 'three-tools-unsigned-nonrelease.exe'
$note = Join-Path $runner ('akula-video-tools-private-note-' + $tag + '.md')
if ((Test-Path -LiteralPath $work) -or (Test-Path -LiteralPath $root) -or
    (Test-Path -LiteralPath $note)) {
  throw 'VIDEO_TOOLS_01A_HOLD: disposable target already exists'
}
$makensis = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS/makensis.exe'
Assert-RealFile $makensis
$installed = $false
try {
  New-Item -ItemType Directory -Force -Path $stage | Out-Null
  Set-Content -LiteralPath $note -Value ('owner-private-' + $tag) -NoNewline
  $noteDigest = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
  foreach ($name in $inputs.Keys) {
    Copy-Item -LiteralPath $inputs[$name] -Destination (Join-Path $stage $name) -ErrorAction Stop
    if ((Get-FileHash -LiteralPath (Join-Path $stage $name) -Algorithm SHA256).Hash.ToLowerInvariant() -cne $digests[$name]) {
      throw 'VIDEO_TOOLS_01A_HOLD: stage digest differs from source'
    }
  }
  $staged = @(Get-ChildItem -LiteralPath $stage -Force)
  if ($staged.Count -ne 3 -or @($staged | Where-Object { $_.Name -cnotin @($inputs.Keys) }).Count -ne 0) {
    throw 'VIDEO_TOOLS_01A_HOLD: staging has unexpected bytes or sidecars'
  }
  & $makensis ("/DLAB_ID=" + $tag) ("/DLAB_STAGE=" + $stage) ("/DLAB_OUTPUT=" + $installer) (Join-Path $repo 'scripts/video-tools-01a-isolated-lab.nsi') | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01A_HOLD: NSIS compilation failed' }
  Assert-RealFile $installer
  $installRun = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
  if ($installRun.ExitCode -ne 0) { throw 'VIDEO_TOOLS_01A_HOLD: three-PE NSIS install failed' }
  $installed = $true
  if (!(Test-Path -LiteralPath $root -PathType Container)) {
    throw 'VIDEO_TOOLS_01A_HOLD: protected installation root missing'
  }
  Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
  $installedItems = @(Get-ChildItem -LiteralPath $root -Force)
  $exact = @($inputs.Keys) + @('uninstall.exe')
  if ($installedItems.Count -ne 4 -or
      @($installedItems | Where-Object { $_.Name -cnotin $exact }).Count -ne 0) {
    throw 'VIDEO_TOOLS_01A_HOLD: installation not isolated to three PE plus uninstaller'
  }
  foreach ($name in $inputs.Keys) {
    $file = Join-Path $root $name
    Assert-RealFile $file
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $digests[$name]) {
      throw 'VIDEO_TOOLS_01A_HOLD: installed PE digest mismatch'
    }
  }
  foreach ($forbidden in @('akula-app.exe','shiva-pdf-preview.exe')) {
    if (Test-Path -LiteralPath (Join-Path $root $forbidden)) {
      throw 'VIDEO_TOOLS_01A_HOLD: separate tools installer modified stock application'
    }
  }
  # VIDEO-TOOLS-01d: check the exact root AND each installed object before any use.
  . (Join-Path $repo 'scripts/video-tools-01d-acl.ps1')
  Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag
  $acl = Get-Acl -LiteralPath $root
  foreach ($ace in $acl.Access) {
    $sid = try { $ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { '' }
    if ($sid -in @('S-1-1-0','S-1-5-11','S-1-5-32-545') -and
        $ace.AccessControlType -eq 'Allow' -and
        (($ace.FileSystemRights -band [Security.AccessControl.FileSystemRights]::WriteData) -ne 0 -or
         ($ace.FileSystemRights -band [Security.AccessControl.FileSystemRights]::Delete) -ne 0)) {
      throw 'VIDEO_TOOLS_01A_HOLD: ordinary users can replace protected media PE'
    }
  }
  # Real negative DACL mutations on the disposable package, always restored.
  $aclProof=@(& (Join-Path $repo 'scripts/windows-video-tools-01d-acl-negative.ps1') -Root $root -LabId $tag 2>&1)
  $aclProof | Out-Host
  if (!(($aclProof | Out-String).Contains('VIDEO_TOOLS_01D_ACL_MUTATION_PASS'))) {
    throw 'VIDEO_TOOLS_01D_HOLD: genuine poisoned Windows ACL was not rejected'
  }
  Write-Output 'VIDEO_TOOLS_01D_ACL_MUTATION_PASS restored=1 release=0'
  # VIDEO-TOOLS-01e: native Rust Win32 ACL admission/poison on real installed PEs.
  $nativeProof=@(& (Join-Path $repo 'scripts/windows-video-tools-01e-native-lab.ps1') -Root $root -LabId $tag -Work $work 2>&1)
  $nativeProof | Out-Host
  if (!(($nativeProof | Out-String).Contains('VIDEO_TOOLS_01E_NATIVE_WIN32_PASS'))) {
    throw 'VIDEO_TOOLS_01E_HOLD: genuine native Windows ACL witness missing'
  }
  Write-Output 'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS release=0'
  # VIDEO-TOOLS-01f: real local standard user must be denied seven Win32 mutation rights.
  $userProof=@(& (Join-Path $repo 'scripts/windows-video-tools-01f-standard-user.ps1') -Root $root -LabId $tag -Work $work 2>&1)
  $userProof | Out-Host
  if (!(($userProof | Out-String).Contains('VIDEO_TOOLS_01F_NONADMIN_PASS'))) {
    throw 'VIDEO_TOOLS_01F_HOLD: genuine standard-user Windows denial missing'
  }
  Write-Output 'VIDEO_TOOLS_01F_NONADMIN_PASS release=0'
  # A foreign file must survive explicit uninstall, rather than Delete /r.
  Set-Content -LiteralPath (Join-Path $root 'unrelated-private-note.md') -Value 'preserve-me' -NoNewline
  $uninstaller = Join-Path $root 'uninstall.exe'
  Assert-RealFile $uninstaller
  $uninstallRun = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
  if ($uninstallRun.ExitCode -ne 0) { throw 'VIDEO_TOOLS_01A_HOLD: uninstall failed' }
  if (!(Test-Path -LiteralPath (Join-Path $root 'unrelated-private-note.md') -PathType Leaf)) {
    throw 'VIDEO_TOOLS_01A_HOLD: foreign private note deleted by uninstall'
  }
  foreach ($name in $exact) {
    if (Test-Path -LiteralPath (Join-Path $root $name)) {
      throw 'VIDEO_TOOLS_01A_HOLD: installed executable survived uninstall'
    }
  }
  if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteDigest) {
    throw 'VIDEO_TOOLS_01A_HOLD: independent user note was changed'
  }
  Remove-Item -LiteralPath (Join-Path $root 'unrelated-private-note.md')
  Remove-Item -LiteralPath $root
  $installed = $false
  Write-Output 'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS protected=1 old_app_excluded=1 exact_three=1 sha256=3 manual_uninstall=1 private_notes_preserved=1 release=0'
} finally {
  if ($installed -and (Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf)) {
    Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
    Start-Process -FilePath (Join-Path $root 'uninstall.exe') -ArgumentList '/S' -Wait | Out-Null
  }
  if (Test-Path -LiteralPath $root) {
    Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
    # Refuse recursive deletion of unknown protected/user files.
    $remaining = @(Get-ChildItem -LiteralPath $root -Force)
    if ($remaining.Count -eq 0) { Remove-Item -LiteralPath $root }
  }
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
  if (Test-Path -LiteralPath $note) { Remove-Item -LiteralPath $note -Force }
}
