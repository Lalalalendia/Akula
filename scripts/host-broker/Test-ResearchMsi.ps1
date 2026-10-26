# Disposable Windows hosted-runner-only smoke, NEVER on a user workstation.
# It tests real per-machine MSI install/uninstall but does NOT sign/release.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Msi,
    [Parameter(Mandatory = $true)][string]$BuiltBrokerExe
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_OS -ne "Windows" -or
    -not $env:RUNNER_TEMP -or -not $env:GITHUB_WORKSPACE) {
    throw "Research MSI destructive test requires an ephemeral GitHub Windows runner"
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "MSI service test must be elevated"
}
$name = "AkulaHostBrokerResearch"
$installDir = Join-Path $env:ProgramFiles $name
$installedExe = Join-Path $installDir "akula-host-broker-lab.exe"
$markerDir = Join-Path $env:ProgramData $name
$setup = (Resolve-Path -LiteralPath $Msi).Path
$source = (Resolve-Path -LiteralPath $BuiltBrokerExe).Path
# Destructive cleanup is only permitted after ALL reserved names are absent.
if ((Get-Service -Name $name -ErrorAction SilentlyContinue) -or
    (Test-Path -LiteralPath $installDir) -or
    (Test-Path -LiteralPath $markerDir)) {
    throw "Existing Broker service or namespace; never replace on CI"
}

function Invoke-Msi([string]$Operation, [string]$Package, [string]$LogName) {
    $log = Join-Path $env:RUNNER_TEMP $LogName
    $arguments = "/$Operation " + '"' + $Package + '"' + " /qn /norestart /l*v " + '"' + $log + '"'
    $process = Start-Process -FilePath "$env:WINDIR\System32\msiexec.exe" -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $log) {
            Get-Content -LiteralPath $log -Tail 45 | Out-Host
        }
        throw "msiexec $Operation failed with exit $($process.ExitCode)"
    }
}
function Has-WeakWrite([string]$Path) {
    $acl = Get-Acl -LiteralPath $Path
    $weak = @("S-1-1-0", "S-1-5-32-545", "S-1-5-11", "S-1-5-32-546")
    $mask = [int][System.Security.AccessControl.FileSystemRights]::Write -bor
            [int][System.Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor
            [int][System.Security.AccessControl.FileSystemRights]::Delete -bor
            [int][System.Security.AccessControl.FileSystemRights]::ChangePermissions -bor
            [int][System.Security.AccessControl.FileSystemRights]::TakeOwnership
    foreach ($ace in $acl.Access) {
        if ($ace.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) { continue }
        # Windows may expose read-only package identities not resolvable
        # on this runner. They cannot replace an executable. Only inspect
        # SIDs that actually have ANY write/delete/owner/DACL rights.
        if (([int]$ace.FileSystemRights -band $mask) -eq 0) { continue }
        try {
            $sid = $ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        } catch {
            Write-Host "Unknown writable ACE on $Path : $($ace.IdentityReference)"
            return $true  # unresolvable WRITER remains fail-closed
        }
        $trustedWriters = @(
            "S-1-5-18",       # LocalSystem
            "S-1-5-32-544",   # Built-in Administrators
            "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464" # TrustedInstaller
        )
        if ($sid -in $weak -or $sid -notin $trustedWriters) {
            Write-Host "Unexpected writable ACE on $Path : $sid"
            return $true
        }
    }
    return $false
}
$installed = $false
try {
    Invoke-Msi -Operation "i" -Package $setup -LogName "akula-host-install-01.log"
    $installed = $true
    $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$name'"
    if ($null -eq $service) { throw "MSI did not register the broker service" }
    if ($service.StartMode -ne "Manual" -or $service.State -ne "Stopped" -or
        $service.StartName -ne "LocalSystem" -or $service.ProcessId -ne 0) {
        throw "Service not manual/stopped/LocalSystem"
    }
    # Account identity alone is insufficient: SCM must enable the
    # specific service SID that the protected Broker custody checks demand.
    $sidType = (& "$env:WINDIR\System32\sc.exe" qsidtype $name | Out-String)
    if ($LASTEXITCODE -ne 0 -or $sidType -notmatch "UNRESTRICTED") {
        throw "MSI omitted required dedicated service SID"
    }
    if (!(Test-Path -LiteralPath $installedExe -PathType Leaf)) {
        throw "Service PE not installed under Program Files"
    }
    if ((Get-FileHash -LiteralPath $installedExe).Hash -ne (Get-FileHash -LiteralPath $source).Hash) {
        throw "Installed PE differs from the isolated compiled Broker"
    }
    $expectedPath = '"' + $installedExe + '" --service-custody-pipe'
    if ($service.PathName -ne $expectedPath) {
        throw "SCM ImagePath not an exact pinned Program Files path: $($service.PathName)"
    }
    foreach ($path in @($installDir, $installedExe)) {
        $item = Get-Item -LiteralPath $path
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
            (Has-WeakWrite -Path $path)) {
            throw "Service executable or directory is reparse/ordinary-user writable: $path"
        }
    }
    if (Test-Path -LiteralPath $markerDir) {
        throw "MSI illegally created a Host Broker authority registration"
    }
    # A separately installed service is not allowed to bootstrap itself.
    # No service start or issuance is attempted at install time.
    Write-Host "AKAI_HOST_MSI_INSTALL_PASS: perMachine, manual LocalSystem, Program Files, no untrusted write, no enrolled registry"
}
finally {
    if ($installed) {
        Invoke-Msi -Operation "x" -Package $setup -LogName "akula-host-uninstall-01.log"
        if (Get-Service -Name $name -ErrorAction SilentlyContinue) {
            throw "Service survived MSI uninstall"
        }
        if (Test-Path -LiteralPath $installedExe) {
            throw "Executable survived MSI uninstall"
        }
        if (Test-Path -LiteralPath $markerDir) {
            throw "Uninstall touched/created an unowned authority marker"
        }
        Write-Host "AKAI_HOST_MSI_UNINSTALL_PASS: independent SCM entry and PE removed"
    }
}
