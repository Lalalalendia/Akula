# RELEASE-WINDOWS-NSIS-UPGRADE-01: independently compiled local-only 0.3.10.
# The pinned checkout is always restored. No GitHub Release is created.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:AKULA_WINDOWS_DISPOSABLE_UPGRADE_LAB -ne '1') {
  throw 'CI-only second NSIS build is disabled outside an ephemeral Windows runner'
}
$native = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app/native/Cargo.toml'
$lock = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app/Cargo.lock'
$config = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app/native/packaging/akula-windows.json'
$dist = Join-Path $env:GITHUB_WORKSPACE 'dist'
$out = Join-Path $dist 'e2e-v2'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$binDir = Join-Path $env:GITHUB_WORKSPACE '.artifacts/cargo-native/release'
$v1bin = Join-Path $binDir 'akula-app.exe'
$v1installers = @(Get-ChildItem -LiteralPath $dist -Filter 'Akula_0.3.9_*-setup.exe' -File)
if ($v1installers.Count -ne 1) { throw 'One already tested 0.3.9 Akula NSIS installer required' }
(Get-FileHash -LiteralPath $v1bin -Algorithm SHA256).Hash |
  Set-Content -LiteralPath (Join-Path $dist 'e2e-v1-executable-sha256.txt')
$old = 'version = "0.3.9"'
$new = 'version = "0.3.10"'
$lf = [string][char]10
$crlf = [string][char]13 + $lf

try {
  $source = [IO.File]::ReadAllText($native)
  if (($source.Split([string[]]@($old), [StringSplitOptions]::None)).Count -ne 2) {
    throw 'Native Cargo package version 0.3.9 must occur exactly once'
  }
  [IO.File]::WriteAllText($native, $source.Replace($old, $new))
  $locked = [IO.File]::ReadAllText($lock).Replace($crlf, $lf)
  $needle = '[[package]]' + $lf + 'name = "aquilum-native"' + $lf + $old
  if (!$locked.Contains($needle)) { throw 'Pinned native Cargo.lock package entry is missing' }
  [IO.File]::WriteAllText($lock, $locked.Replace($needle, $needle.Replace($old, $new)))

  $product = Get-Content -LiteralPath $config -Raw | ConvertFrom-Json
  if ($product.productName -ne 'Akula' -or $product.version -ne '0.3.9') {
    throw 'Packager is not the independent Akula 0.3.9 NSIS configuration'
  }
  $product.version = '0.3.10'
  $product | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $config -Encoding utf8NoBOM
  Push-Location (Join-Path $env:GITHUB_WORKSPACE 'aquilum-app')
  try {
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or
        @($metadata.packages | Where-Object { $_.name -eq 'aquilum-native' -and $_.version -eq '0.3.10' }).Count -ne 1) {
      throw 'Distinct v2 package metadata does not match locked Rust workspace'
    }
    cargo build --release --locked -p aquilum-native --features installed
    if ($LASTEXITCODE -ne 0) { throw 'Real 0.3.10 native application build failed' }
    $nativeBin = Join-Path $binDir 'aquilum-native.exe'
    Copy-Item -LiteralPath $nativeBin -Destination $v1bin -Force
    cargo packager -p aquilum-native --release --formats nsis --config native/packaging/akula-windows.json --out-dir $out
    if ($LASTEXITCODE -ne 0) { throw 'Real 0.3.10 NSIS packager failed' }
  } finally {
    Pop-Location
  }
  $setups = @(Get-ChildItem -LiteralPath $out -Filter '*-setup.exe' -File)
  if ($setups.Count -ne 1 -or $setups[0].Name -notmatch '^(Akula|akula-app)_0\.3\.10_') {
    throw 'Exactly one Akula 0.3.10 setup with approved identity is required'
  }
  if ($setups[0].Name.StartsWith('akula-app_')) {
    Rename-Item -LiteralPath $setups[0].FullName -NewName ($setups[0].Name -replace '^akula-app_', 'Akula_')
  }
  $current = (Get-FileHash -LiteralPath $v1bin -Algorithm SHA256).Hash
  $previous = (Get-Content -LiteralPath (Join-Path $dist 'e2e-v1-executable-sha256.txt') -Raw).Trim()
  if ($current -eq $previous) { throw 'CI v2 reused identical compiled executable bytes' }
  $current | Set-Content -LiteralPath (Join-Path $out 'executable-sha256.txt')
  $v2installer = @(Get-ChildItem -LiteralPath $out -Filter 'Akula_0.3.10_*-setup.exe' -File)
  if ($v2installer.Count -ne 1) { throw 'Distinct v2 installer missing after normalization' }
  @(
    "PINNED_GIT_SHA=$env:GITHUB_SHA"
    'V1=0.3.9'
    'V2=0.3.10-CI-only-variant'
    "V1_EXE_SHA256=$previous"
    "V2_EXE_SHA256=$current"
    "V1_NSIS_SHA256=$((Get-FileHash -LiteralPath $v1installers[0].FullName -Algorithm SHA256).Hash)"
    "V2_NSIS_SHA256=$((Get-FileHash -LiteralPath $v2installer[0].FullName -Algorithm SHA256).Hash)"
  ) | Set-Content -LiteralPath (Join-Path $out 'PROVENANCE-NOT-RELEASE.txt')
  Write-Host 'AKULA_WINDOWS_TWO_REAL_NSIS_BUILDS_PASS v1=0.3.9 v2=0.3.10 no_release=1'
} finally {
  Push-Location $env:GITHUB_WORKSPACE
  try {
    git restore -- aquilum-app/native/Cargo.toml aquilum-app/Cargo.lock aquilum-app/native/packaging/akula-windows.json
    if ($LASTEXITCODE -ne 0) { throw 'Failed to restore original sources after CI-only v2' }
  } finally { Pop-Location }
}
