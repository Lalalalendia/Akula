# Disposable Windows CI-only installer-like ACL fixture; no P2P authority.
param([switch]$JoinedPipe)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

$name = "AkulaHostBrokerResearch"
$root = "C:\ProgramData\AkulaHostBrokerResearch"
$marker = Join-Path $root "authority.marker"
$exe = (Resolve-Path "aquilum-app/host-broker-lab/target/debug/akula-host-broker-lab.exe").Path
if (Get-Service -Name $name -ErrorAction SilentlyContinue) {
    throw "existing SCM service: refusing replacement"
}
if (Test-Path -LiteralPath $root) {
    throw "existing installed namespace: refusing replacement"
}
$createdService = $false
$createdRoot = $false
function Expect-Refusal([string] $arg) {
    & $exe $arg *> $null
    if ($LASTEXITCODE -eq 0) { throw "'$arg' unexpectedly accepted untrusted state" }
}
function Expect-Success([string] $arg) {
    & $exe $arg
    if ($LASTEXITCODE -ne 0) { throw "'$arg' refused protected source" }
}
function Must-Icacls([string[]] $aclArguments) {
    & icacls.exe @aclArguments | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls failure: $($aclArguments -join ' ')" }
}
function Expect-Stop() {
    $service = Get-Service -Name $name -ErrorAction Stop
    if ($service.Status -ne "Stopped") {
        Stop-Service -Name $name -ErrorAction Stop
        (Get-Service -Name $name).WaitForStatus("Stopped", [TimeSpan]::FromSeconds(15))
    }
}
try {
    Expect-Refusal "--verify-custody"
    $mode = if ($JoinedPipe) { '--service-custody-pipe' } else { '--service-custody' }
    $command = '"' + $exe + '" ' + $mode
    New-Service -Name $name -BinaryPathName $command -StartupType Manual -DisplayName "Akula Custody Research (ephemeral)" | Out-Null
    $createdService = $true
    & sc.exe sidtype $name unrestricted | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "service SID installation failed" }

    # A LocalSystem service may NOT provision a missing authority marker.
    $missingRefused = $false
    try { Start-Service -Name $name -ErrorAction Stop }
    catch { $missingRefused = $true }
    if (-not $missingRefused) { throw "missing installer-owned marker was accepted" }
    (Get-Service -Name $name).WaitForStatus("Stopped", [TimeSpan]::FromSeconds(12))
    if (Test-Path -LiteralPath $root) { throw "service illegally auto-provisioned marker" }

    New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
    $createdRoot = $true
    Expect-Refusal "--verify-custody"
    [System.IO.File]::WriteAllText($marker, "AKULA-CUSTODY-ONLY;NOT-A-GRANT")
    $identity = "NT SERVICE\AkulaHostBrokerResearch"
    $system = "*S-1-5-18"
    $admin = "*S-1-5-32-544"

    Must-Icacls -aclArguments @($root, "/inheritance:r")
    Must-Icacls -aclArguments @($root, "/grant:r", "$($system):(OI)(CI)F", "$($admin):(OI)(CI)F", "$($identity):(OI)(CI)F")
    Must-Icacls -aclArguments @($root, "/setowner", $admin)
    Must-Icacls -aclArguments @($marker, "/inheritance:r")
    Must-Icacls -aclArguments @($marker, "/grant:r", "$($system):F", "$($admin):F", "$($identity):F")
    Must-Icacls -aclArguments @($marker, "/setowner", $admin)

    Expect-Success "--verify-custody"
    Start-Service -Name $name -ErrorAction Stop
    (Get-Service -Name $name).WaitForStatus("Running", [TimeSpan]::FromSeconds(15))
    $running = Get-CimInstance Win32_Service -Filter "Name='$name'"
    if ($running.StartName -ne "LocalSystem" -or $running.ProcessId -eq 0) {
        throw "SCM did not start expected LocalSystem service"
    }
    Expect-Success "--verify-scm"
    Expect-Success "--verify-custody"
    if ($JoinedPipe) {
        # Service PID, SYSTEM token and enabled service SID must match the
        # live named-pipe process while pinned custody handles are held.
        Expect-Success "--verify-pipe"
        Expect-Success "--verify-pipe"
    } else {
        Expect-Refusal "--verify-pipe"
    }

    # Even an otherwise valid marker must fail if ordinary Users gain write.
    Must-Icacls -aclArguments @($marker, "/grant:r", "*S-1-5-32-545:W")
    Expect-Refusal "--verify-custody"
    if ($JoinedPipe) {
        # A real connection wakes the overlapped denial-only service.
        # It must detect changed source ACL itself and STOP: deliberately
        # do not manually Stop-Service or the test would prove nothing.
        & $exe "--verify-pipe" *> $null
        try {
            (Get-Service -Name $name).WaitForStatus("Stopped", [TimeSpan]::FromSeconds(12))
        } catch {
            throw "running Broker did not stop after live marker ACL tampering"
        }
    } else {
        Expect-Stop
    }
    $tamperedRefused = $false
    try { Start-Service -Name $name -ErrorAction Stop }
    catch { $tamperedRefused = $true }
    if (-not $tamperedRefused) { throw "SCM accepted user-write policy marker" }
    (Get-Service -Name $name).WaitForStatus("Stopped", [TimeSpan]::FromSeconds(12))
    Expect-Refusal "--verify-pipe"

    Must-Icacls -aclArguments @($marker, "/remove:g", "*S-1-5-32-545")
    Expect-Success "--verify-custody"
    Start-Service -Name $name -ErrorAction Stop
    (Get-Service -Name $name).WaitForStatus("Running", [TimeSpan]::FromSeconds(15))
    Expect-Success "--verify-custody"
    if ($JoinedPipe) { Expect-Success "--verify-pipe" }
    Expect-Stop
}
finally {
    if ($createdService) {
        try { Expect-Stop } catch { Write-Warning "SCM cleanup stop failed: $_" }
        & sc.exe delete $name | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "ephemeral service cleanup failed" }
    }
    if ($createdRoot) {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
    }
}
