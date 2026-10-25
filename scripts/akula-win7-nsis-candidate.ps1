# Package only independently PE-audited Windows 7-target binaries, as one
# ordinary Akula NSIS candidate. Never publishes or signs a release.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Win7 NSIS candidate packaging requires a clean hosted Windows runner'
}
$root = $env:GITHUB_WORKSPACE
$trial = Join-Path $root 'win7-lab-output'
$exe = Join-Path $trial 'akula-app.exe'
$sidecar = Join-Path $trial 'shiva-pdf-preview.exe'
$mainAuditPath = Join-Path $trial 'pe-audit/full-audit.json'
$sidecarAuditPath = Join-Path $root 'win7-full-results/shiva-audit/full-audit.json'
foreach ($file in @($exe, $sidecar, $mainAuditPath, $sidecarAuditPath)) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing Win7 LAB evidence: $file" }
}
$mainAudit = Get-Content -LiteralPath $mainAuditPath -Raw | ConvertFrom-Json
$sidecarAudit = Get-Content -LiteralPath $sidecarAuditPath -Raw | ConvertFrom-Json
foreach ($entry in @(@{name='Akula'; path=$exe; audit=$mainAudit},
                     @{name='Shiva'; path=$sidecar; audit=$sidecarAudit})) {
    $actual = (Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $entry.audit.binary_sha256.ToLowerInvariant()) {
        throw "$($entry.name) SHA256 does not match its independent PE report"
    }
    if (@($entry.audit.known_post_win7_imports).Count -ne 0) {
        throw "$($entry.name) still has known Windows 8+ imports"
    }
    if (@($entry.audit.potentially_risky_imported_dlls) -contains 'combase.dll') {
        throw "$($entry.name) directly imports combase.dll"
    }
    if ($entry.audit.runtime_tested -eq $true) {
        throw 'A static PE audit must not claim Windows 7 runtime was verified'
    }
}
$ole = @($mainAudit.imports.'ole32.dll')
if ($ole -notcontains 'CoCreateFreeThreadedMarshaler' -or $ole -notcontains 'CoTaskMemFree') {
    throw 'Akula Win7 COM remapping is not backed by the exact OLE32 imports'
}
$packagerConfig = Get-Content -LiteralPath (Join-Path $root 'aquilum-app/native/packaging/akula-windows.json') -Raw | ConvertFrom-Json
$native = Get-Content -LiteralPath (Join-Path $root 'aquilum-app/native/Cargo.toml') -Raw
$sourceVersion = [regex]::Match($native, '(?m)^version\s*=\s*"([^"]+)"')
if (!$sourceVersion.Success -or $sourceVersion.Groups[1].Value -cne $packagerConfig.version) {
    throw 'Akula release version and NSIS config are not identical'
}
$staging = Join-Path $root '.artifacts/cargo-native/release'
New-Item -ItemType Directory -Force -Path $staging | Out-Null
$stagedExe = Join-Path $staging 'akula-app.exe'
$stagedSidecar = Join-Path $staging 'shiva-pdf-preview.exe'
Copy-Item -LiteralPath $exe -Destination $stagedExe -Force
Copy-Item -LiteralPath $sidecar -Destination $stagedSidecar -Force
$expectedExeHash = (Get-FileHash -LiteralPath $stagedExe -Algorithm SHA256).Hash
$expectedSidecarHash = (Get-FileHash -LiteralPath $stagedSidecar -Algorithm SHA256).Hash

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Push-Location (Join-Path $root 'aquilum-app')
try {
    & cargo packager -p aquilum-native --release --formats nsis --config native/packaging/akula-windows.json --out-dir $dist
    if ($LASTEXITCODE -ne 0) { throw 'Win7-target NSIS packaging failed' }
} finally {
    Pop-Location
}
if ((Get-FileHash -LiteralPath $stagedExe -Algorithm SHA256).Hash -cne $expectedExeHash -or
    (Get-FileHash -LiteralPath $stagedSidecar -Algorithm SHA256).Hash -cne $expectedSidecarHash) {
    throw 'cargo-packager modified the PE-audited Win7 binaries'
}
$setups = @(Get-ChildItem -LiteralPath $dist -Filter '*-setup.exe' -File)
if ($setups.Count -ne 1) { throw "Expected one Akula NSIS; found $($setups.Count)" }
if ($setups[0].Name -cne "Akula_$($packagerConfig.version)_x64-setup.exe") {
    if ($setups[0].Name -cne "akula-app_$($packagerConfig.version)_x64-setup.exe") {
        throw "Unexpected NSIS identity: $($setups[0].Name)"
    }
    Rename-Item -LiteralPath $setups[0].FullName -NewName ($setups[0].Name -replace '^akula-app_', 'Akula_')
}
# NSIS itself must not demand a newer OS before it can install the Akula EXE.
$installer = Join-Path $dist "Akula_$($packagerConfig.version)_x64-setup.exe"
if (!(Test-Path -LiteralPath $installer -PathType Leaf)) { throw 'Audited NSIS installer missing' }
$vswhere = Join-Path ([Environment]::GetEnvironmentVariable('ProgramFiles(x86)')) 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsroot = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
if (!$vsroot) { throw 'Cannot audit Win7 installer without MSVC tools' }
$toolsets = @(Get-ChildItem (Join-Path $vsroot 'VC/Tools/MSVC') -Directory | Sort-Object Name -Descending)
if ($toolsets.Count -eq 0) { throw 'No MSVC x64 toolset for NSIS audit' }
$dumpbin = Join-Path $toolsets[0].FullName 'bin/Hostx64/x64/dumpbin.exe'
$peDir = Join-Path $dist 'win7-installer-pe-audit'
New-Item -ItemType Directory -Force -Path $peDir | Out-Null
$importsFile = Join-Path $peDir 'imports.txt'
& $dumpbin /imports $installer | Set-Content -LiteralPath $importsFile -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect the exact Akula NSIS PE imports' }
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
$length = (Get-Item -LiteralPath $installer).Length
& python .github/scripts/akula-win7-dumpbin-audit.py --dumpbin $importsFile --out $peDir --sha256 $hash --bytes $length
if ($LASTEXITCODE -ne 0) { throw 'NSIS static import scanner failed' }
$pe = Get-Content -LiteralPath (Join-Path $peDir 'full-audit.json') -Raw | ConvertFrom-Json
if (@($pe.known_post_win7_imports).Count -ne 0) {
    throw 'Ordinary Akula NSIS installer imports Windows 8+ APIs'
}
Write-Host "AKULA_WIN7_NORMAL_NSIS_PACKAGED_PASS version=$($packagerConfig.version) windows7_runtime_tested=false no_release=true"
