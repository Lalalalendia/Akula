# AKULA-PUBLISHED-DIRECT-JUMP-01: isolated public NSIS install/upgrade proof.
# This tests one-step installation, not an unattended in-app updater click.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:AKULA_DISPOSABLE_DIRECT_JUMP -ne '1' -or
    $env:GITHUB_REPOSITORY -ne 'Lalalalendia/Akula') {
  throw 'AKULA_DIRECT_JUMP_HOLD: disposable GitHub Windows only'
}
$repo = $env:GITHUB_REPOSITORY
$old = '0.3.9'
$next = '0.3.12'
$oldSource = '8746cd28c13e2fdcee7e61c6ab7f7b336f1d740e'
$nextSource = 'fa6ec577e55c4ace245c38130cb5ddb07d0c61a3'
$oldHash = 'a1b27f6e5c79cdedbfe659c6b2c6c3e502b229e985748668cda1e4e731ce48d4'
$nextHash = '7bae191d8a9046bee0f10700e3274489ec9da2964d7517fc01dd0adddf335577'
$base = Join-Path $env:RUNNER_TEMP 'akula-published-039-to-0312'
if (Test-Path -LiteralPath $base) { throw 'Direct-jump workspace preexists' }
$oldDir = Join-Path $base 'old'
$nextDir = Join-Path $base 'new'
New-Item -ItemType Directory -Force -Path $oldDir, $nextDir | Out-Null
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
foreach ($p in @($install, $state, $reg)) {
  if (Test-Path -LiteralPath $p) { throw "Runner is not clean: $p" }
}

function Fetch-Release([string]$v, [string]$source, [string]$pin, [string]$dir, [bool]$full) {
  $tag = 'v' + $v
  $r = gh api "repos/$repo/releases/tags/$tag" | ConvertFrom-Json
  if ($LASTEXITCODE -ne 0 -or $r.tag_name -cne $tag -or
      $r.target_commitish -cne $source -or $r.draft) {
    throw "Release tag/source mismatch: $tag"
  }
  $name = 'Akula_' + $v + '_x64-setup.exe'
  $assets = @($r.assets | Where-Object { $_.name -ceq $name })
  if ($assets.Count -ne 1 -or $assets[0].digest -cne "sha256:$pin") {
    throw "Release installer identity mismatch: $tag"
  }
  if ($full) {
    $names = @($r.assets | ForEach-Object { $_.name } | Sort-Object) -join '|'
    $expected = @($name, "$name.sig", 'latest.json', 'latest.json.sig' | Sort-Object) -join '|'
    if ($names -cne $expected) { throw 'Expected exactly four signed release assets' }
    gh release download $tag --repo $repo --dir $dir
  } else {
    gh release download $tag --repo $repo --dir $dir --pattern $name
  }
  if ($LASTEXITCODE -ne 0) { throw "Cannot download public release $tag" }
  $path = Join-Path $dir $name
  if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Downloaded NSIS absent: $tag" }
  $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($hash -cne $pin -or (Get-Item -LiteralPath $path).Length -ne $assets[0].size) {
    throw "NSIS bytes mismatch: $tag"
  }
  if ($full) {
    $manifestPath = Join-Path $dir 'latest.json'
    $metaAsset = @($r.assets | Where-Object { $_.name -ceq 'latest.json' })
    if ($metaAsset.Count -ne 1) { throw 'Missing signed release manifest' }
    $manifestHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($metaAsset[0].digest -cne "sha256:$manifestHash") { throw 'Manifest digest mismatch' }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $win = $manifest.artifacts.'windows-x86_64'
    if ($manifest.product -cne 'Akula' -or $manifest.version -cne $next -or
        $manifest.source_commit -cne $nextSource -or $win.filename -cne $name -or
        $win.sha256 -cne $pin -or $win.size -ne $assets[0].size) {
      throw 'Published latest.json fails exact artifact identity'
    }
    foreach ($sig in @("$name.sig", 'latest.json.sig')) {
      if (!(Test-Path -LiteralPath (Join-Path $dir $sig) -PathType Leaf)) {
        throw "Missing signature file: $sig"
      }
    }
  }
  return $path
}

# No v0.3.10 or v0.3.11 bytes are ever downloaded or executed.
$oldSetup = Fetch-Release $old $oldSource $oldHash $oldDir $false
$nextSetup = Fetch-Release $next $nextSource $nextHash $nextDir $true
if (@(Get-ChildItem -LiteralPath $base -Recurse -Filter '*-setup.exe').Count -ne 2) {
  throw 'Unexpected intermediate installer present'
}

# Independent upstream Aquilum witness.
$legacy = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
if (Test-Path -LiteralPath $legacy) { throw 'Legacy state preexists' }
New-Item -ItemType Directory -Path $legacy -Force | Out-Null
$witness = Join-Path $legacy 'do-not-touch.txt'
Set-Content -LiteralPath $witness -Encoding UTF8 -Value 'AQUILUM-UNMODIFIED'
$witnessHash = (Get-FileHash -LiteralPath $witness -Algorithm SHA256).Hash

$vault = Join-Path $base 'vault'
New-Item -ItemType Directory -Path $vault -Force | Out-Null
$note = Join-Path $vault 'Upgrade.md'
Set-Content -LiteralPath $note -Encoding UTF8 -Value '# Preserve Akula notes across direct update'
$noteHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
function Assert-UserData {
  if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash -or
      (Get-FileHash -LiteralPath $witness -Algorithm SHA256).Hash -ne $witnessHash) {
    throw 'NSIS modified user or upstream files'
  }
}
function Invoke-NSIS([string]$setup, [string[]]$args) {
  $p = Start-Process -FilePath $setup -ArgumentList $args -PassThru
  if (!$p.WaitForExit(180000)) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    throw "NSIS timeout: $setup"
  }
  if ($p.ExitCode -ne 0) { throw "NSIS failed with $($p.ExitCode): $setup" }
}
function Assert-Installed([string]$version) {
  if (!(Test-Path -LiteralPath $exe -PathType Leaf) -or !(Test-Path -LiteralPath $reg)) {
    throw "Installed Akula missing: $version"
  }
  $actual = (Get-ItemProperty -Path $reg -Name DisplayVersion).DisplayVersion
  if ($actual -cne $version) { throw "Installed $actual, expected $version" }
}
function Wait-Gui([int]$excluded = -1) {
  for ($i = 0; $i -lt 55; $i++) {
    $app = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
      Where-Object { $_.MainWindowHandle -ne 0 -and $_.Id -ne $excluded })
    if ($app.Count -gt 0) { return $app[0] }
    Start-Sleep -Seconds 1
  }
  throw 'Akula GUI did not open'
}

Invoke-NSIS $oldSetup @('/S')
Assert-Installed $old
$openArgs = '--vault "' + $vault + '" --open "' + $note + '"'
Start-Process -FilePath $exe -ArgumentList $openArgs | Out-Null
$original = Wait-Gui
Assert-UserData
Write-Host 'AKULA_PUBLISHED_039_GUI_PASS'

Invoke-NSIS $nextSetup @('/S', '/R', '/UPDATE', '/ARGS')
Assert-Installed $next
$upgraded = Wait-Gui $original.Id
Assert-UserData
Write-Host 'AKULA_PUBLISHED_DIRECT_JUMP_PASS from=0.3.9 to=0.3.12 steps=1 intermediates=0 gui=1 notes=1 sha256=1'

# Uninstall and confirm Akula and Aquilum user-owned data remain unmodified.
foreach ($p in @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue)) {
  $p.Refresh()
  if ($p.MainWindowHandle -ne 0) { $null = $p.CloseMainWindow() }
  if (!$p.WaitForExit(25000)) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    throw 'Akula did not exit cleanly'
  }
}
$uninstaller = Join-Path $install 'uninstall.exe'
if (!(Test-Path -LiteralPath $uninstaller -PathType Leaf)) { throw 'Missing Akula uninstaller' }
Invoke-NSIS $uninstaller @('/S')
for ($i = 0; $i -lt 60 -and ((Test-Path $exe) -or (Test-Path $reg)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path $exe) -or (Test-Path $reg) -or !(Test-Path $state -PathType Container)) {
  throw 'Uninstaller did not preserve Akula state or remove installed software'
}
Assert-UserData
Write-Host 'AKULA_PUBLISHED_DIRECT_JUMP_CLEANUP_PASS uninstall=1 appdata=1 notes=1'
