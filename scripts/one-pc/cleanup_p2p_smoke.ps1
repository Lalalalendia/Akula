param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d{8}-\d{9}$')]
    [string]$RunId
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifactRoot = Join-Path $repo '.artifacts'
$runRoot = Join-Path (Join-Path $artifactRoot 'one-pc-p2p-smoke') $RunId
$summaryPath = Join-Path $runRoot 'summary.json'
$identityBinary = Join-Path $artifactRoot 'cargo-p2p-harness\debug\akula-p2p-identity.exe'
$prefix = 'Lalalalendia/Akula/P2P/LocalHarness/'

if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
    throw "P2P smoke summary was not found: $summaryPath"
}
if (-not (Test-Path -LiteralPath $identityBinary -PathType Leaf)) {
    throw "Debug-only identity cleanup helper was not found: $identityBinary"
}
if (@(Get-Process -Name aquilum-native -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'Close all Akula windows before cleanup. No process or credential was changed.'
}

$summary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
if ($summary.RunId -ne $RunId -or -not $summary.TestCredentialTargets) {
    throw 'Run ID or test-target manifest in the summary is invalid.'
}

foreach ($instance in @('A', 'B')) {
    $target = [string]$summary.TestCredentialTargets.$instance
    $suffix = $target.Substring([Math]::Min($prefix.Length, $target.Length))
    $validTarget = $target.StartsWith($prefix, [StringComparison]::Ordinal) `
        -and $target.Length -le 256 `
        -and $suffix -match '^[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*$'
    if (-not $validTarget) { throw "Refusing an out-of-namespace test target for profile $instance." }

    $envName = 'AKULA_P2P_TEST_CREDENTIAL_TARGET'
    $previous = [Environment]::GetEnvironmentVariable($envName, 'Process')
    try {
        [Environment]::SetEnvironmentVariable($envName, $target, 'Process')
        $cleanupOutput = @(& $identityBinary '--test-cleanup' 2>&1)
        $cleanupExit = $LASTEXITCODE
        $cleanupText = ($cleanupOutput | ForEach-Object { $_.ToString() }) -join "`n"
        Set-Content -LiteralPath (Join-Path $runRoot "cleanup-$instance.stdout.log") -Value $cleanupText -Encoding UTF8
        if ($cleanupExit -ne 0 -or $cleanupText.Trim() -ne 'TEST_IDENTITY_CLEANED') {
            throw "Could not clean the temporary identity for profile $instance."
        }

        $readbackOutput = @(& $identityBinary '--id' 2>&1)
        $readbackExit = $LASTEXITCODE
        $readbackText = ($readbackOutput | ForEach-Object { $_.ToString() }) -join "`n"
        Set-Content -LiteralPath (Join-Path $runRoot "cleanup-$instance-readback.log") -Value $readbackText -Encoding UTF8
        if ($readbackExit -eq 0 -or $readbackText -notmatch 'device identity has not been initialized') {
            throw "Credential Manager entry for profile $instance still exists after cleanup."
        }
    } finally {
        [Environment]::SetEnvironmentVariable($envName, $previous, 'Process')
    }
}

$summary.CleanupStatus = 'PASS: both per-run test identities were deleted and verified absent'
$summary | Add-Member -MemberType NoteProperty -Name CleanupRecoveryAt `
    -Value ((Get-Date).ToString('o')) -Force
$summary | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
Write-Output "P2P smoke credentials cleaned; summary: $summaryPath"
