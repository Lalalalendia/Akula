param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('A', 'B')]
    [string]$Instance,

    [string]$Executable,

    [string]$P2PCredentialTarget
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$instanceRoot = Join-Path $repo '.artifacts\one-pc-instances'
if (-not $Executable) {
    $Executable = Join-Path $repo '.artifacts\cargo-native\debug\aquilum-native.exe'
}
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "Akula binary not found: $Executable. Build it before launching a profile."
}

$profilePath = Join-Path (Join-Path $instanceRoot $Instance) 'profile.json'
if (-not (Test-Path -LiteralPath $profilePath -PathType Leaf)) {
    throw "Profile is not prepared: $profilePath. Run prepare_instances.py first."
}
$profile = Get-Content -LiteralPath $profilePath -Raw | ConvertFrom-Json
foreach ($key in @('app_data_dir', 'vault_root')) {
    $path = [IO.Path]::GetFullPath([string]$profile.$key)
    $prefix = [IO.Path]::GetFullPath($instanceRoot) + [IO.Path]::DirectorySeparatorChar
    if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Profile path escapes the isolated fixture root: $path"
    }
    if (-not (Test-Path -LiteralPath $path -PathType Container)) {
        throw "Profile directory does not exist: $path"
    }
}

if ($P2PCredentialTarget) {
    $prefix = 'Lalalalendia/Akula/P2P/LocalHarness/'
    $suffix = $P2PCredentialTarget.Substring([Math]::Min($prefix.Length, $P2PCredentialTarget.Length))
    $expectedHarness = Join-Path $repo '.artifacts\cargo-p2p-harness\debug\aquilum-native.exe'
    $validTarget = $P2PCredentialTarget.StartsWith($prefix, [StringComparison]::Ordinal) `
        -and $P2PCredentialTarget.Length -le 256 `
        -and $suffix -match '^[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*$'
    if (-not $validTarget) {
        throw 'P2P test credential target is outside the local harness namespace.'
    }
    if ([IO.Path]::GetFullPath($Executable) -ne [IO.Path]::GetFullPath($expectedHarness)) {
        throw "P2P test identities can only be passed to the dedicated debug harness binary: $expectedHarness"
    }
}

# Do not close or replace another Akula process. Each process receives explicit,
# separately prepared app data and vault paths.
$arguments = @(
    '--data-dir', [string]$profile.app_data_dir,
    '--vault', [string]$profile.vault_root,
    '--size', '1280x800'
)
$targetEnvName = 'AKULA_P2P_TEST_CREDENTIAL_TARGET'
$previousTarget = [Environment]::GetEnvironmentVariable($targetEnvName, 'Process')
try {
    if ($P2PCredentialTarget) {
        [Environment]::SetEnvironmentVariable($targetEnvName, $P2PCredentialTarget, 'Process')
    }
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -PassThru
} finally {
    [Environment]::SetEnvironmentVariable($targetEnvName, $previousTarget, 'Process')
}
Write-Output "instance=$Instance profile=$($profile.profile_id) pid=$($process.Id)"
Write-Output "app_data_dir=$($profile.app_data_dir)"
Write-Output "vault_root=$($profile.vault_root)"
if ($P2PCredentialTarget) { Write-Output "p2p_test_target=$P2PCredentialTarget" }
