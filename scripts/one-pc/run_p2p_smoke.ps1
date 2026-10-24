param(
    [ValidateRange(1, 90)]
    [int]$TimeoutMinutes = 20
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$app = Join-Path $repo 'aquilum-app'
$artifactRoot = Join-Path $repo '.artifacts'
$profilesRoot = Join-Path $artifactRoot 'one-pc-instances'
$targetManifestPath = Join-Path $profilesRoot 'p2p-test-targets.json'
$targetDir = Join-Path $artifactRoot 'cargo-p2p-harness'
$binary = Join-Path $targetDir 'debug\aquilum-native.exe'
$identityBinary = Join-Path $targetDir 'debug\akula-p2p-identity.exe'
$devCmd = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat'
$instanceRunner = Join-Path $PSScriptRoot 'run_instance.ps1'
$runId = Get-Date -Format 'yyyyMMdd-HHmmssfff'
$runRoot = Join-Path (Join-Path $artifactRoot 'one-pc-p2p-smoke') $runId
[void](New-Item -ItemType Directory -Path $runRoot -Force)

if (-not (Test-Path -LiteralPath $devCmd -PathType Leaf)) {
    throw "Visual Studio developer environment was not found: $devCmd"
}
if (-not (Test-Path -LiteralPath $targetManifestPath -PathType Leaf)) {
    throw "P2P test targets are not prepared. Run scripts/one-pc/prepare_instances.py first."
}

$targetManifest = Get-Content -LiteralPath $targetManifestPath -Raw | ConvertFrom-Json
$targetPrefix = 'Lalalalendia/Akula/P2P/LocalHarness/'
$targetA = [string]$targetManifest.targets.A + "/$runId"
$targetB = [string]$targetManifest.targets.B + "/$runId"
foreach ($target in @($targetA, $targetB)) {
    $suffix = $target.Substring([Math]::Min($targetPrefix.Length, $target.Length))
    $validTarget = $target.StartsWith($targetPrefix, [StringComparison]::Ordinal) `
        -and $target.Length -le 256 `
        -and $suffix -match '^[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*$'
    if (-not $validTarget) {
        throw "Invalid ephemeral P2P test target in $targetManifestPath"
    }
}
if ($targetA -eq $targetB) { throw 'The P2P test targets must be distinct.' }

$summary = [ordered]@{
    RunId = $runId
    StartedAt = (Get-Date).ToString('o')
    GitBranch = (& git -C $repo branch --show-current).Trim()
    GitHead = (& git -C $repo rev-parse HEAD).Trim()
    RustToolchain = '1.96.0-x86_64-pc-windows-msvc'
    TestTargetNamespace = $targetPrefix
    TestCredentialTargets = @{ A = $targetA; B = $targetB }
    EndpointIds = @{}
    Processes = @{}
    Checks = @{}
    CleanupStatus = 'pending'
    Status = 'FAIL'
    Failure = $null
    NotPhysicalDeviceOrNetworkAcceptance = $true
}
$summaryPath = Join-Path $runRoot 'summary.json'
$buildOutput = Join-Path $runRoot 'build.stdout.log'
$buildError = Join-Path $runRoot 'build.stderr.log'
$processIds = New-Object 'System.Collections.Generic.List[int]'
$failure = $null
$overallStatus = 'FAIL'
$cleanupAllowed = $true
$targetsTouched = $false

function Save-Summary {
    $summary.CompletedAt = (Get-Date).ToString('o')
    $summary.Status = $overallStatus
    $summary.Failure = $failure
    $summary | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
}

function Invoke-IdentityCommand {
    param(
        [Parameter(Mandatory = $true)][string]$CredentialTarget,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string]$LogName
    )

    $stdout = Join-Path $runRoot "$LogName.stdout.log"
    $stderr = Join-Path $runRoot "$LogName.stderr.log"
    $envName = 'AKULA_P2P_TEST_CREDENTIAL_TARGET'
    $previous = [Environment]::GetEnvironmentVariable($envName, 'Process')
    try {
        [Environment]::SetEnvironmentVariable($envName, $CredentialTarget, 'Process')
        $output = @(& $identityBinary $Command 2>&1)
        $exitCode = $LASTEXITCODE
    } finally {
        [Environment]::SetEnvironmentVariable($envName, $previous, 'Process')
    }
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    Set-Content -LiteralPath $stdout -Value $text -Encoding UTF8
    [pscustomobject]@{ ExitCode = $exitCode; Output = $text; Stdout = $stdout; Stderr = $stderr }
}

function Get-EndpointId {
    param([string]$CredentialTarget, [string]$Command, [string]$LogName)
    $result = Invoke-IdentityCommand -CredentialTarget $CredentialTarget -Command $Command -LogName $LogName
    if ($result.ExitCode -ne 0) {
        throw "$Command failed for a test target; inspect $($result.Stdout). $($result.Output)"
    }
    $id = $result.Output.Trim()
    if ($id -notmatch '^[0-9a-fA-F]{64}$') {
        throw "$Command did not return a 64-character public EndpointId; inspect $($result.Stdout)."
    }
    return $id.ToLowerInvariant()
}

try {
    $buildCommand = Join-Path $runRoot 'build.cmd'
    $buildLines = @(
        '@echo off',
        "call `"$devCmd`" -no_logo -arch=x64 -host_arch=x64",
        'if errorlevel 1 exit /b %errorlevel%',
        "set `"CARGO_TARGET_DIR=$targetDir`"",
        "cd /d `"$app`"",
        'cargo +1.96.0 check --locked -p aquilum-core --features p2p-device-identity',
        'if errorlevel 1 exit /b %errorlevel%',
        'cargo +1.96.0 test --locked -p aquilum-core --lib --features p2p-device-identity-test-harness local_harness_target_is_confined_to_its_namespace',
        'if errorlevel 1 exit /b %errorlevel%',
        'cargo +1.96.0 build --locked -p aquilum-native --features "p2p-device-contacts-preview aquilum-core/p2p-device-identity-test-harness"',
        'if errorlevel 1 exit /b %errorlevel%',
        'cargo +1.96.0 build --locked -p aquilum-core --features p2p-device-identity-test-harness --bin akula-p2p-identity',
        'exit /b %errorlevel%'
    )
    Set-Content -LiteralPath $buildCommand -Value $buildLines -Encoding ASCII
    $build = Start-Process -FilePath $env:ComSpec -ArgumentList @('/d', '/c', $buildCommand) `
        -WorkingDirectory $repo -NoNewWindow -PassThru `
        -RedirectStandardOutput $buildOutput -RedirectStandardError $buildError
    $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
    while (-not $build.WaitForExit(1000)) {
        if ((Get-Date) -ge $deadline) {
            & taskkill.exe /PID $build.Id /T /F | Out-Null
            throw "Debug harness build timed out after $TimeoutMinutes minutes."
        }
    }
    if ($build.ExitCode -ne 0) {
        throw "Debug harness build failed with exit code $($build.ExitCode); inspect $buildOutput and $buildError."
    }
    if (-not (Select-String -LiteralPath $buildOutput -Pattern 'test result: ok\. 1 passed; 0 failed' -Quiet)) {
        throw "The credential-target boundary test did not run exactly one passing test; inspect $buildOutput."
    }
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf) -or -not (Test-Path -LiteralPath $identityBinary -PathType Leaf)) {
        throw 'The debug harness build did not produce both expected binaries.'
    }
    $summary.Checks.ProductionIdentityCompile = 'PASS (test harness feature disabled)'
    $summary.Checks.CredentialTargetBoundaryTest = 'PASS (test executed)'
    $summary.Checks.Build = 'PASS'

    $targetsTouched = $true
    $endpointA = Get-EndpointId -CredentialTarget $targetA -Command '--init' -LogName 'identity-A-init'
    $endpointB = Get-EndpointId -CredentialTarget $targetB -Command '--init' -LogName 'identity-B-init'
    if ($endpointA -eq $endpointB) { throw 'The isolated test credential targets returned the same EndpointId.' }
    $summary.EndpointIds = @{ A = $endpointA; B = $endpointB }
    $summary.Checks.DistinctEndpointIds = 'PASS'
    Save-Summary

    foreach ($instance in @('A', 'B')) {
        $target = if ($instance -eq 'A') { $targetA } else { $targetB }
        $launchText = (& $instanceRunner -Instance $instance -Executable $binary `
            -P2PCredentialTarget $target 2>&1 | Out-String)
        Add-Content -LiteralPath (Join-Path $runRoot 'gui.stdout.log') -Value $launchText -Encoding UTF8
        $match = [regex]::Match($launchText, 'pid=(\d+)')
        if (-not $match.Success) { throw "Could not start profile $instance. $launchText" }
        $pidValue = [int]$match.Groups[1].Value
        $processIds.Add($pidValue)
        $process = Get-Process -Id $pidValue -ErrorAction Stop
        $summary.Processes[$instance] = @{ Pid = $pidValue; StartTime = $process.StartTime.ToString('o') }
        Save-Summary
    }

    $windowDeadline = (Get-Date).AddSeconds(40)
    $windowsReady = $false
    while ((Get-Date) -lt $windowDeadline) {
        $windows = @($processIds | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
        if ($windows.Count -eq 2 -and @($windows | Where-Object { $_.MainWindowHandle -eq [IntPtr]::Zero }).Count -eq 0) {
            $windowsReady = $true
            break
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $windowsReady) { throw 'Both test-profile Akula windows did not become ready within 40 seconds.' }

    $profileManifests = @{}
    foreach ($instance in @('A', 'B')) {
        $profile = Get-Content -LiteralPath (Join-Path $profilesRoot "$instance\profile.json") -Raw | ConvertFrom-Json
        $profileManifests[$instance] = $profile
        foreach ($file in @('ui-state.sqlite3', 'documents.sqlite3', 'device-id')) {
            $path = Join-Path $profile.app_data_dir $file
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                throw "Profile $instance did not create its own $file."
            }
        }
    }
    if ($profileManifests.A.app_data_dir -eq $profileManifests.B.app_data_dir -or $profileManifests.A.vault_root -eq $profileManifests.B.vault_root -or $profileManifests.A.loopback_endpoint_hint -eq $profileManifests.B.loopback_endpoint_hint) {
        throw 'The local profile data, vault, and loopback address hints must all be distinct.'
    }
    $deviceA = (Get-FileHash -Algorithm SHA256 (Join-Path $profileManifests.A.app_data_dir 'device-id')).Hash
    $deviceB = (Get-FileHash -Algorithm SHA256 (Join-Path $profileManifests.B.app_data_dir 'device-id')).Hash
    if ($deviceA -eq $deviceB) { throw 'The app-data device identifiers unexpectedly match.' }
    $readbackA = Get-EndpointId -CredentialTarget $targetA -Command '--id' -LogName 'identity-A-readback'
    $readbackB = Get-EndpointId -CredentialTarget $targetB -Command '--id' -LogName 'identity-B-readback'
    if ($readbackA -ne $endpointA -or $readbackB -ne $endpointB) {
        throw 'The process-level identity readback did not match the initialized test identities.'
    }
    $summary.Checks.DistinctEndpointIds = 'PASS'
    $summary.Checks.DistinctProfilesAndVaults = 'PASS'
    $summary.Checks.DistinctAddressHints = 'PASS (configured hints; no P2P listener is present on main)'
    $summary.Checks.SeparateAppData = 'PASS'
    $summary.Checks.CrossProcessCredentialReadback = 'PASS'
    $summary.AppDeviceIdHashes = @{ A = $deviceA; B = $deviceB }
    $overallStatus = 'PASS'
} catch {
    $failure = $_.Exception.Message
    $summary.Checks.Failure = $failure
    $errorPath = Join-Path $runRoot 'run.stderr.log'
    Add-Content -LiteralPath $errorPath -Value $failure -Encoding UTF8
} finally {
    foreach ($processId in $processIds) {
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($process) { [void]$process.CloseMainWindow() }
    }
    foreach ($processId in $processIds) {
        try { Wait-Process -Id $processId -Timeout 20 -ErrorAction Stop } catch {
            # Wait-Process can report "not found" when the window closed just
            # before it began waiting. Only retain a cleanup failure if this
            # PID is still running after the wait attempt.
            $remaining = Get-Process -Id $processId -ErrorAction SilentlyContinue
            if ($remaining) {
                $cleanupAllowed = $false
                $cleanupError = "Akula process $processId did not close gracefully; it was left running."
                Add-Content -LiteralPath (Join-Path $runRoot 'run.stderr.log') -Value $cleanupError -Encoding UTF8
                if (-not $failure) { $failure = $cleanupError }
                $overallStatus = 'FAIL'
            }
        }
    }

    if (-not $targetsTouched) {
        $summary.CleanupStatus = 'NOT_NEEDED: no test identities were created'
    } elseif ($cleanupAllowed -and (Test-Path -LiteralPath $identityBinary -PathType Leaf)) {
        foreach ($instance in @('A', 'B')) {
            $target = if ($instance -eq 'A') { $targetA } else { $targetB }
            $cleanup = Invoke-IdentityCommand -CredentialTarget $target `
                -Command '--test-cleanup' -LogName "identity-$instance-cleanup"
            if ($cleanup.ExitCode -ne 0 -or $cleanup.Output.Trim() -ne 'TEST_IDENTITY_CLEANED') {
                $cleanupAllowed = $false
                $cleanupError = "Could not clean up test Credential Manager target for profile $instance."
                Add-Content -LiteralPath (Join-Path $runRoot 'run.stderr.log') -Value $cleanupError -Encoding UTF8
                if (-not $failure) { $failure = $cleanupError }
                $overallStatus = 'FAIL'
            }
        }
        if ($cleanupAllowed) {
            foreach ($instance in @('A', 'B')) {
                $target = if ($instance -eq 'A') { $targetA } else { $targetB }
                $readback = Invoke-IdentityCommand -CredentialTarget $target `
                    -Command '--id' -LogName "identity-$instance-after-cleanup"
                if ($readback.ExitCode -eq 0 -or $readback.Output -notmatch 'device identity has not been initialized') {
                    $cleanupAllowed = $false
                    $cleanupError = "Credential Manager target for profile $instance still exists after cleanup."
                    Add-Content -LiteralPath (Join-Path $runRoot 'run.stderr.log') -Value $cleanupError -Encoding UTF8
                    if (-not $failure) { $failure = $cleanupError }
                    $overallStatus = 'FAIL'
                }
            }
        }
        if ($cleanupAllowed) { $summary.CleanupStatus = 'PASS: both per-run test identities were deleted and verified absent' }
    } else {
        $summary.CleanupStatus = 'PENDING: a process remains open or cleanup helper is unavailable; see run.stderr.log'
    }
    Save-Summary
}

Write-Output "P2P local smoke $overallStatus; summary: $summaryPath"
if ($overallStatus -ne 'PASS') {
    throw "P2P local smoke failed. Inspect retained artifacts under $runRoot."
}
