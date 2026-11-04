# PC-DESK-07: only on disposable GitHub-hosted Windows.
# Published 0.3.17 same-version reinstall, manual 0.3.16 rollback and return.
# No physical power loss, simulated power loss, auto-rollback or cancellation claim.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -cne 'true' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:AKULA_DESK07_DISPOSABLE -cne '1' -or
    $env:GITHUB_EVENT_NAME -cnotin @('pull_request', 'workflow_dispatch')) {
  throw 'AKULA_DESK07_HOLD: disposable GitHub-hosted Windows required'
}

$versions = @{
  '0.3.16' = @{
    source = 'c18380449982d3bcf962abb863da4c27dd10375d'
    sha256 = '680408c0fa3b69d0df444195e24b0fbf1773d4921ad8cf11f781722f79798d75'
    size = 22408502
  }
  '0.3.17' = @{
    source = '3effbfbe957c91b7dfa3f63af2f30614230d0798'
    sha256 = '9da2de42ea8808455f234479cac3ce574ab903095f9223f9ccb2640ceab10170'
    size = 22411384
  }
}
$base = Join-Path $env:RUNNER_TEMP ('akula-desk07-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $base | Out-Null

$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$legacyInstall = Join-Path $env:LOCALAPPDATA 'Aquilum'
$legacyData = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
$legacyReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
$runReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

foreach ($path in @($install, $registry, $state, $legacyInstall, $legacyData, $legacyReg)) {
  if (Test-Path -LiteralPath $path) { throw "AKULA_DESK07_HOLD: dirty runner $path" }
}
if ((Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue)) {
  throw 'AKULA_DESK07_HOLD: preexisting Akula process'
}
if (Get-ItemProperty -Path $runReg -Name Aquilum -ErrorAction SilentlyContinue) {
  throw 'AKULA_DESK07_HOLD: preexisting Aquilum autorun value'
}

# Synthetic, separately hashed user and Aquilum sentinels: never touch real data.
New-Item -ItemType Directory -Path $state, $legacyInstall, $legacyData | Out-Null
$stateFile = Join-Path $state 'desk07-user-state.txt'
$upstreamInstall = Join-Path $legacyInstall 'legacy-do-not-touch.txt'
$upstreamData = Join-Path $legacyData 'user-data-do-not-touch.txt'
$vault = Join-Path $base 'vault'
New-Item -ItemType Directory -Path $vault | Out-Null
$note = Join-Path $vault 'Keep.md'
[IO.File]::WriteAllText($stateFile, 'AKULA-PC-DESK07-USER')
[IO.File]::WriteAllText($upstreamInstall, 'AQUILUM-PC-DESK07-INSTALL')
[IO.File]::WriteAllText($upstreamData, 'AQUILUM-PC-DESK07-APPDATA')
[IO.File]::WriteAllText($note, '# Keep this exact Markdown source')
New-Item -Path $legacyReg -Force | Out-Null
Set-ItemProperty -Path $legacyReg -Name DisplayName -Value 'Aquilum'
New-Item -Path $runReg -Force | Out-Null
Set-ItemProperty -Path $runReg -Name Aquilum -Value 'aquilum-do-not-touch'
$digests = @{}
foreach ($path in @($stateFile, $upstreamInstall, $upstreamData, $note)) {
  $digests[$path] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}
function Assert-Witnesses {
  foreach ($path in $digests.Keys) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf) -or
        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -cne $digests[$path]) {
      throw "Akula changed a user/upstream witness: $path"
    }
  }
  if ((Get-ItemProperty $legacyReg -Name DisplayName).DisplayName -cne 'Aquilum' -or
      (Get-ItemProperty $runReg -Name Aquilum).Aquilum -cne 'aquilum-do-not-touch') {
    throw 'Akula changed upstream Aquilum registry witnesses'
  }
}
function Test-Pin([string]$path, [string]$sha256, [long]$size) {
  if (!(Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
  if ((Get-Item -LiteralPath $path).Length -ne $size) { return $false }
  return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $sha256
}
function Get-PinnedInstaller([string]$version) {
  $pin = $versions[$version]
  $name = 'Akula_' + $version + '_x64-setup.exe'
  $release = Invoke-RestMethod -Uri ('https://api.github.com/repos/Lalalalendia/Akula/releases/tags/v' + $version) -Headers @{
    'Accept' = 'application/vnd.github+json'
    'User-Agent' = 'Akula-PC-DESK07'
  }
  if ($release.tag_name -cne ('v' + $version) -or $release.draft -or
      $release.prerelease -or $release.target_commitish -cne $pin.source) {
    throw "Published Akula release provenance changed: $version"
  }
  $assets = @($release.assets | Where-Object { $_.name -ceq $name })
  $signatures = @($release.assets | Where-Object { $_.name -ceq ($name + '.sig') })
  if ($assets.Count -ne 1 -or $signatures.Count -ne 1 -or
      $signatures[0].size -le 0 -or $assets[0].size -ne $pin.size -or
      $assets[0].digest -cne ('sha256:' + $pin.sha256) -or
      $assets[0].browser_download_url -cne ('https://github.com/Lalalalendia/Akula/releases/download/v' + $version + '/' + $name)) {
    throw "Published Akula asset identity/signature record changed: $version"
  }
  $target = Join-Path $base $name
  Invoke-WebRequest -Uri $assets[0].browser_download_url -OutFile $target -MaximumRetryCount 3 -RetryIntervalSec 3
  if (!(Test-Pin $target $pin.sha256 $pin.size)) { throw "Downloaded Akula package hash mismatch: $version" }
  return $target
}
function Invoke-PinnedNSIS([string]$setup, [string]$sha256, [long]$size, [string[]]$flags) {
  # This check is deliberately before Start-Process: corrupt bytes are never run.
  if (!(Test-Pin $setup $sha256 $size)) { throw 'Installer pin rejected before execution' }
  $p = Start-Process -FilePath $setup -ArgumentList $flags -PassThru
  if (!$p.WaitForExit(180000)) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    throw "NSIS timed out: $setup"
  }
  $p.Refresh()
  if ($p.ExitCode -ne 0) { throw "NSIS exit $($p.ExitCode): $setup" }
}
function Assert-Installed([string]$version) {
  if (!(Test-Path -LiteralPath $exe -PathType Leaf) -or !(Test-Path $registry)) {
    throw "Akula $version not installed"
  }
  if ((Get-ItemProperty -Path $registry -Name DisplayVersion).DisplayVersion -cne $version) {
    throw "Akula registry version differs from $version"
  }
  $entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
    Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -ceq 'Akula' })
  if ($entries.Count -ne 1) { throw "Expected exactly one Akula uninstall identity, found $($entries.Count)" }
  if (!(Test-Path -LiteralPath (Join-Path $install 'shiva-pdf-preview.exe') -PathType Leaf)) {
    throw 'Installed Akula Shiva sidecar missing'
  }
  Assert-Witnesses
}
function Assert-GuiLaunch {
  $args = '--vault "' + $vault + '" --open "' + $note + '"'
  $started = Start-Process -FilePath $exe -ArgumentList $args -PassThru
  $visible = $null
  for ($i=0; $i -lt 70; $i++) {
    $windows = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
      Where-Object { $_.MainWindowHandle -ne 0 })
    if ($windows.Count -gt 0) { $visible = $windows[0]; break }
    if ($started.HasExited) { throw 'Akula exited before opening installed GUI' }
    Start-Sleep -Seconds 1
  }
  if ($null -eq $visible) { throw 'Akula installed GUI did not appear' }
  if (!$visible.CloseMainWindow() -or !$visible.WaitForExit(25000)) {
    throw 'Akula installed GUI did not close normally'
  }
  Assert-Witnesses
}

$v16 = Get-PinnedInstaller '0.3.16'
$v17 = Get-PinnedInstaller '0.3.17'
Write-Host 'AKULA_DESK07_PUBLISHED_PIN_PASS versions=0.3.16,0.3.17 sha256=1 signature_records=1'

# The negative fixture never executes: the same entrypoint rejects bad bytes.
$bad = Join-Path $base 'tampered-not-executed.exe'
Copy-Item -LiteralPath $v17 -Destination $bad
$stream = [IO.File]::Open($bad, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite)
try {
  $stream.Seek(-1, [IO.SeekOrigin]::End) | Out-Null
  $last = $stream.ReadByte()
  $stream.Seek(-1, [IO.SeekOrigin]::End) | Out-Null
  $stream.WriteByte([byte]($last -bxor 1))
} finally {
  $stream.Dispose()
}
$refused = $false
try {
  Invoke-PinnedNSIS $bad $versions['0.3.17'].sha256 $versions['0.3.17'].size @('/S')
} catch {
  if ($_.Exception.Message -cne 'Installer pin rejected before execution') { throw }
  $refused = $true
}
if (!$refused) { throw 'Tampered published installer was not rejected' }
Write-Host 'AKULA_DESK07_TAMPER_PREFLIGHT_PASS mutated_exe_executed=0'

Invoke-PinnedNSIS $v17 $versions['0.3.17'].sha256 $versions['0.3.17'].size @('/S')
Assert-Installed '0.3.17'
$exe17 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
$sidecar17 = (Get-FileHash -LiteralPath (Join-Path $install 'shiva-pdf-preview.exe') -Algorithm SHA256).Hash
Invoke-PinnedNSIS $v17 $versions['0.3.17'].sha256 $versions['0.3.17'].size @('/S')
Assert-Installed '0.3.17'
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -cne $exe17 -or
    (Get-FileHash -LiteralPath (Join-Path $install 'shiva-pdf-preview.exe') -Algorithm SHA256).Hash -cne $sidecar17) {
  throw 'Same-version reinstall changed installed binary identity'
}
Write-Host 'AKULA_DESK07_REINSTALL_PASS version=0.3.17 duplicate=0 binaries=1 state=1 aquilum=1'

# Manual rollback is an explicit signed previous-version reinstall,
# NOT automatic rollback and NOT interrupted NSIS transaction recovery.
Invoke-PinnedNSIS $v16 $versions['0.3.16'].sha256 $versions['0.3.16'].size @('/S','/R','/UPDATE','/ARGS')
Assert-Installed '0.3.16'
$exe16 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
if ($exe16 -ceq $exe17) { throw 'Manual rollback did not replace the v0.3.17 application binary' }
Assert-GuiLaunch
Write-Host 'AKULA_DESK07_MANUAL_ROLLBACK_PASS from=0.3.17 to=0.3.16 gui=1 state=1 notes=1 aquilum=1 automatic=0'

Invoke-PinnedNSIS $v17 $versions['0.3.17'].sha256 $versions['0.3.17'].size @('/S','/R','/UPDATE','/ARGS')
Assert-Installed '0.3.17'
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -cne $exe17) {
  throw 'Re-upgrade did not restore pinned v0.3.17 application binary'
}
Assert-GuiLaunch
Write-Host 'AKULA_DESK07_REUPGRADE_PASS from=0.3.16 to=0.3.17 gui=1 sha256=1 state=1'

$uninstall = Join-Path $install 'uninstall.exe'
if (!(Test-Path -LiteralPath $uninstall -PathType Leaf)) { throw 'Akula uninstaller missing' }
$p = Start-Process -FilePath $uninstall -ArgumentList @('/S') -PassThru
if (!$p.WaitForExit(180000)) { throw 'Akula uninstaller timed out' }
$p.Refresh()
if ($p.ExitCode -ne 0) { throw "Akula uninstaller exit $($p.ExitCode)" }
for ($i=0; $i -lt 40 -and ((Test-Path $exe) -or (Test-Path $registry)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path $exe) -or (Test-Path $registry) -or !(Test-Path $state)) {
  throw 'Akula uninstall left product identity or removed user state'
}
Assert-Witnesses
Write-Host 'AKULA_DESK07_UNINSTALL_PASS state=1 notes=1 aquilum=1'
