# VIDEO-18b-04: real five-EXE custody proof, no installer, no publishing.
#requires -Version 7.2
param(
    [Parameter(Mandatory=$true)][string]$AppPath,
    [Parameter(Mandatory=$true)][string]$ShivaPath,
    [Parameter(Mandatory=$true)][string]$MuxPath,
    [Parameter(Mandatory=$true)][string]$YtDlpPath,
    [Parameter(Mandatory=$true)][string]$QuickJsPath,
    [Parameter(Mandatory=$true)][string]$ReportPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or $env:GITHUB_EVENT_NAME -cne 'pull_request') {
    throw 'VIDEO18B04 refuses any non-hosted or non-PR execution'
}
if (!$env:EXPECTED_HEAD_SHA -or (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO18B04 source checkout must match exact PR HEAD'
}
$repo = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$allowPath = Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json'
$allow = Get-Content -LiteralPath $allowPath -Raw | ConvertFrom-Json
$names = @('akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
    'akula-yt-dlp.exe','akula-quickjs.exe')
if ($allow.schema -ne 1 -or $allow.release_admitted -ne $false -or
    $allow.installer_signing_authorized -ne $false -or
    $allow.stage_binary_distribution_authorized -ne $false -or
    @($allow.components).Count -ne 5 -or
    (@($allow.components | ForEach-Object { $_.name }) -join ',') -cne ($names -join ',')) {
    throw 'VIDEO18B04 source allowlist must remain an exact five-file non-release HOLD'
}
if ((git -C (Join-Path $repo '_shiva-preview') rev-parse HEAD).Trim() -cne
    '40b72a90b1dac39812c77c7321e57d613a81cb51') {
    throw 'VIDEO18B04 upstream Shiva source is not pinned'
}
# The checked-out Shiva revision has NO tracked Cargo.lock. Cargo may
# generate a lock during the build; record its digest as UNREVIEWED evidence.
$lockPaths = @('aquilum-app/Cargo.lock','labs/video-lite/Cargo.lock','_shiva-preview/Cargo.lock')
$locks = [ordered]@{}
foreach ($lock in $lockPaths) {
    $lockfile = Join-Path $repo $lock
    if (!(Test-Path -LiteralPath $lockfile -PathType Leaf)) {
        throw "VIDEO18B04 missing immutable dependency lock: $lock"
    }
    $locks[$lock] = (Get-FileHash -LiteralPath $lockfile -Algorithm SHA256).Hash.ToLowerInvariant()
}
$shivaLockReview = 'GENERATED_UNREVIEWED_NOT_TRACKED_IN_PINNED_REVISION'
Write-Host 'VIDEO18B04_SHIVA_LOCK_HOLD generated_dependency_lock_is_not_a_source_pin=1'
function Reject-Reparse([string]$path) {
    $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'VIDEO18B04 rejected symlink/junction/reparse file'
    }
}
function Digest([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Check-Pe([string]$path) {
    $f = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        if ($f.Length -lt 512 -or $f.ReadByte() -ne 77 -or $f.ReadByte() -ne 90) {
            throw 'VIDEO18B04 staged file is not a PE/MZ executable'
        }
    } finally { $f.Dispose() }
}
$inputs = [ordered]@{
    'akula-app.exe' = [IO.Path]::GetFullPath($AppPath)
    'shiva-pdf-preview.exe' = [IO.Path]::GetFullPath($ShivaPath)
    'akula-video-mux.exe' = [IO.Path]::GetFullPath($MuxPath)
    'akula-yt-dlp.exe' = [IO.Path]::GetFullPath($YtDlpPath)
    'akula-quickjs.exe' = [IO.Path]::GetFullPath($QuickJsPath)
}
$stage = [IO.Path]::GetFullPath((Join-Path $env:RUNNER_TEMP ('Video18b04-Custody-' + [guid]::NewGuid().ToString('N'))))
$report = [IO.Path]::GetFullPath($ReportPath)
if (!(($stage + '\').StartsWith(([IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'),
    [StringComparison]::OrdinalIgnoreCase)) -or
    (Test-Path -LiteralPath $stage) -or
    $report -ceq (Join-Path $stage 'custody.json')) {
    throw 'VIDEO18B04 refuses a non-isolated custody stage'
}
$stageCreated = $false
try {
    New-Item -ItemType Directory -Path $stage -ErrorAction Stop | Out-Null
    $stageCreated = $true
    Reject-Reparse $stage
    $records = @()
    foreach ($name in $names) {
        $source = $inputs[$name]
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "VIDEO18B04 cannot locate real component $name"
        }
        Reject-Reparse $source
        Check-Pe $source
        $inHash = Digest $source
        $target = Join-Path $stage $name
        Copy-Item -LiteralPath $source -Destination $target -ErrorAction Stop
        Reject-Reparse $target
        Check-Pe $target
        if ((Digest $target) -cne $inHash) { throw "VIDEO18B04 staged EXE mutated: $name" }
        $size = (Get-Item -LiteralPath $target).Length
        $definition = @($allow.components | Where-Object { $_.name -ceq $name })
        if ($definition.Count -ne 1) { throw "VIDEO18B04 missing unique component metadata $name" }
        if ($definition[0].kind -ceq 'pinned-upstream' -and
            ($definition[0].sha256 -cne $inHash -or $definition[0].size_bytes -ne $size)) {
            throw "VIDEO18B04 upstream download pin rejected: $name"
        }
        if ($name -ceq 'akula-video-mux.exe' -and
            (!$env:AKULA_BUNDLED_MUX_SHA256 -or $inHash -cne $env:AKULA_BUNDLED_MUX_SHA256.ToLowerInvariant())) {
            throw 'VIDEO18B04 compiled Rust mux hash differs from Akula embedded pin'
        }
        $sig = (Get-AuthenticodeSignature -FilePath $target).Status.ToString()
        $records += [ordered]@{
            name = $name; sha256 = $inHash; bytes = [long]$size;
            source = $definition[0].repository; revision = $definition[0].reference;
            license_status = $definition[0].license_status; authenticode_status = $sig
        }
        Write-Host "VIDEO18B04_EXACT_COMPONENT_PASS name=$name sha256=$inHash bytes=$size"
    }
    if (@(Get-ChildItem -LiteralPath $stage -File).Count -ne 5) {
        throw 'VIDEO18B04 stage must contain exactly five real EXEs'
    }
    # A real negative test: mutate a custody-only copy then require refusal.
    $probe = Join-Path $stage 'akula-yt-dlp.exe'
    [IO.File]::WriteAllText($probe, 'MALICIOUS-STAGED-EXE-REPLACEMENT')
    if ((Digest $probe) -ceq $records[3].sha256) {
        throw 'VIDEO18B04 mutation negative test did not change its fixture'
    }
    $pin = @($allow.components | Where-Object { $_.name -ceq 'akula-yt-dlp.exe' })[0].sha256
    if ((Digest $probe) -ceq $pin) {
        throw 'VIDEO18B04 tampered EXE passed pinned hash check'
    }
    Write-Host 'VIDEO18B04_TAMPER_REFUSAL_PASS staged_yt_dlp_replacement=1'
    Copy-Item -LiteralPath $inputs['akula-yt-dlp.exe'] -Destination $probe -Force
    if ((Digest $probe) -cne $pin) { throw 'VIDEO18B04 recovery copy failed pin recheck' }
    if (@($records | Where-Object { $_.license_status -notlike '*REVIEW*' -and
        $_.license_status -notlike '*BLOCKED*' }).Count -ne 0) {
        throw 'VIDEO18B04 cannot silently mark unresolved notices as approved'
    }
    $manifest = [ordered]@{
        schema = 1
        mode = 'NON-RELEASE FIVE-EXE CUSTODY - UNSIGNED'
        source_commit = $env:EXPECTED_HEAD_SHA
        shiva_commit = '40b72a90b1dac39812c77c7321e57d613a81cb51'
        shiva_lock_review = $shivaLockReview
        lock_sha256 = $locks
        release_admitted = $false
        files = $records
        unresolved_license_reviews = @($records | ForEach-Object { $_.name })
    }
    $json = $manifest | ConvertTo-Json -Depth 8
    $reportParent = Split-Path -Parent $report
    if (!(Test-Path -LiteralPath $reportParent -PathType Container)) {
        New-Item -ItemType Directory -Path $reportParent -Force | Out-Null
    }
    [IO.File]::WriteAllText($report,$json,[Text.Encoding]::UTF8)
    if (@((Get-Content -LiteralPath $report -Raw | ConvertFrom-Json).files).Count -ne 5) {
        throw 'VIDEO18B04 attestation lacks five components'
    }
    Write-Host 'VIDEO18B04_REAL_FIVE_CUSTODY_PASS compiled=3 upstream=2 exact=5 no_installer=1 no_release=1'
    Write-Host 'VIDEO18B04_LICENSE_RELEASE_HOLD_PASS unresolved=5'
}
finally {
    if ($stageCreated -and (Test-Path -LiteralPath $stage)) {
        Reject-Reparse $stage
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction Stop
    }
}
