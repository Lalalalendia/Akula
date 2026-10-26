# Emit auditable checkout and package hashes only after the hosted Windows
# GUI edit/save/restart/uninstall smoke has passed. This is NOT signing.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'Candidate provenance is restricted to GitHub-hosted Windows runners'
}
if ($env:GITHUB_RUN_ID -notmatch '^[0-9]+$' -or $env:GITHUB_RUN_ATTEMPT -notmatch '^[0-9]+$') {
  throw 'Missing GitHub Actions run identity'
}
$source = (git -C $env:GITHUB_WORKSPACE rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $source -notmatch '^[a-f0-9]{40}$') {
  throw 'Candidate source checkout has no valid full commit SHA'
}
# pull_request uses a GitHub-created integration commit. workflow_dispatch
# may check out a separately specified commit, so event SHA is not proof
# of the actual source revision in that case.
if ($env:GITHUB_EVENT_NAME -eq 'pull_request' -and $source -ne $env:GITHUB_SHA) {
  throw 'Pull-request candidate checkout differs from the tested integration SHA'
}
$dist = Join-Path $env:GITHUB_WORKSPACE 'dist'
$setups = @(Get-ChildItem -LiteralPath $dist -Filter 'Akula_*-setup.exe' -File)
if ($setups.Count -ne 1) {
  throw "Expected exactly one Windows beta installer, found $($setups.Count)"
}
$setup = $setups[0]
$binary = Join-Path $env:GITHUB_WORKSPACE '.artifacts/cargo-native/release/akula-app.exe'
$lock = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app/Cargo.lock'
if (!(Test-Path -LiteralPath $binary) -or !(Test-Path -LiteralPath $lock)) {
  throw 'Cannot attribute candidate without built executable and Cargo.lock'
}
$setupHash = (Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$lockHash = (Get-FileHash -LiteralPath $lock -Algorithm SHA256).Hash.ToLowerInvariant()
$manifest = [ordered]@{
  schema = 'akula-windows-beta-candidate-v1'
  actual_checkout_sha = $source
  event_sha = $env:GITHUB_SHA
  event_name = $env:GITHUB_EVENT_NAME
  run_id = $env:GITHUB_RUN_ID
  run_attempt = $env:GITHUB_RUN_ATTEMPT
  installer_name = $setup.Name
  installer_bytes = [long]$setup.Length
  installer_sha256 = $setupHash
  compiled_executable_sha256 = $binaryHash
  cargo_lock_sha256 = $lockHash
  gui_persistence_smoke = 'passed'
  publication = 'CI artifact only; not a GitHub release'
  signing = 'No Authenticode or Akula update signing performed by this workflow'
}
$path = Join-Path $dist 'candidate-manifest.json'
[System.IO.File]::WriteAllText(
  $path,
  ($manifest | ConvertTo-Json -Depth 4),
  [System.Text.UTF8Encoding]::new($false)
)
$roundTrip = [System.IO.File]::ReadAllText($path) | ConvertFrom-Json
if ($roundTrip.actual_checkout_sha -ne $source -or
    $roundTrip.installer_sha256 -ne $setupHash -or
    $roundTrip.compiled_executable_sha256 -ne $binaryHash -or
    $roundTrip.installer_bytes -ne $setup.Length) {
  throw 'Candidate provenance manifest did not round-trip'
}
Write-Host "AKULA_WINDOWS_BETA_PROVENANCE_PASS checkout=$source sha256=$setupHash"
