# Requires a protected release pipeline to supply its independently pinned
# publisher certificate thumbprint. No publisher key/certificate is shipped.
# This is a SIGNATURE GATE, not an installer and not an OS authority grant.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BrokerExe,
    [Parameter(Mandatory = $true)][string]$Msi,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Fa-f]{40}$')]
    [string]$ExpectedPublisherThumbprint
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:OS -ne "Windows_NT") { throw "Authenticode verification is Windows-only" }
if ($ExpectedPublisherThumbprint -match '^0+$') {
    throw "A placeholder fingerprint cannot authorize release"
}

function Assert-Publisher([string]$Path, [string]$Extension) {
    $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    if (-not ($item -is [System.IO.FileInfo])) {
        throw "Expected regular file: $Extension"
    }
    if ($item.Extension -ine $Extension -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Unexpected release input type/alias: $Extension"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $resolved
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $signature.SignerCertificate) {
        throw "Unsigned or untrusted $Extension cannot be released"
    }
    $actual = $signature.SignerCertificate.Thumbprint.Replace(" ", "").ToUpperInvariant()
    if ($actual -cne $ExpectedPublisherThumbprint.ToUpperInvariant()) {
        throw "Unexpected publisher certificate for $Extension"
    }
    [pscustomobject]@{
        Type = $Extension
        Sha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash
        PublisherThumbprint = $actual
    }
}

# Both the PE and MSI require independent valid signatures with the SAME
# release-pinned leaf. This preflight MUST be run in an immutable release
# pipeline; caller-supplied fingerprint alone is NOT a production trust root.
Assert-Publisher -Path $BrokerExe -Extension ".exe"
Assert-Publisher -Path $Msi -Extension ".msi"
