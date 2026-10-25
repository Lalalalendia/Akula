# Build and audit the same native Windows 7-compatible binaries for the
# protected Akula Windows release candidate. No publication or Win7 runtime claim.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or $env:GITHUB_REF -ne 'refs/heads/main') {
    throw 'Publisher Win7 staging requires a protected main workflow on a hosted Windows runner'
}
$checkout = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $checkout -cne $env:GITHUB_SHA) {
    throw 'Win7 publisher staging must use the exact approved main commit'
}
$root = $env:GITHUB_WORKSPACE
$pin = '40b72a90b1dac39812c77c7321e57d613a81cb51'
$shivaActual = (git -C _shiva-preview rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $shivaActual -cne $pin) {
    throw 'PDF companion source changed from the approved Shiva commit'
}
$target = 'x86_64-win7-windows-msvc'
$dir = Join-Path $root 'win7-full-results'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$env:RUSTFLAGS = '--cfg windows_slim_errors'
$env:CARGO_INCREMENTAL = '0'
& rustc +nightly -Vv | Set-Content -LiteralPath (Join-Path $dir 'compiler.txt')
if ($LASTEXITCODE -ne 0) { throw 'Win7 compiler cannot be queried' }

$env:CARGO_TARGET_DIR = Join-Path $root '.artifacts/cargo-native'
& cargo +nightly build --manifest-path aquilum-app/Cargo.toml --release --locked -p aquilum-native --features installed,s3-pair-beta --target $target -Z build-std=std,panic_abort *> (Join-Path $dir 'akula-build.log')
$buildExit = $LASTEXITCODE
Get-Content -LiteralPath (Join-Path $dir 'akula-build.log') -Tail 25 | Write-Host
if ($buildExit -ne 0) { throw "Full Akula Win7 build failed ($buildExit)" }
$exe = Join-Path $env:CARGO_TARGET_DIR "$target/release/aquilum-native.exe"
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Win7 Akula binary missing' }

$env:CARGO_TARGET_DIR = Join-Path $root '.artifacts/shiva-win7'
& cargo +nightly build --manifest-path _shiva-preview/Cargo.toml --release -p cli --target $target -Z build-std=std,panic_abort *> (Join-Path $dir 'shiva-build.log')
$buildExit = $LASTEXITCODE
Get-Content -LiteralPath (Join-Path $dir 'shiva-build.log') -Tail 25 | Write-Host
if ($buildExit -ne 0) { throw "Shiva Win7 build failed ($buildExit)" }
$shiva = Join-Path $env:CARGO_TARGET_DIR "$target/release/shiva.exe"
if (!(Test-Path -LiteralPath $shiva -PathType Leaf)) { throw 'Win7 Shiva binary missing' }

$vswhere = Join-Path ([Environment]::GetEnvironmentVariable('ProgramFiles(x86)')) 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsroot = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
if (!$vsroot) { throw 'MSVC dumpbin tools unavailable' }
$toolset = @(Get-ChildItem (Join-Path $vsroot 'VC/Tools/MSVC') -Directory | Sort-Object Name -Descending)
if ($toolset.Count -lt 1) { throw 'MSVC x64 toolset unavailable' }
$dumpbin = Join-Path $toolset[0].FullName 'bin/Hostx64/x64/dumpbin.exe'
if (!(Test-Path -LiteralPath $dumpbin -PathType Leaf)) { throw 'dumpbin.exe unavailable' }

function Get-VerifiedPeAudit {
    param([string]$Binary, [string]$Output)
    New-Item -ItemType Directory -Force -Path $Output | Out-Null
    $importsFile = Join-Path $Output 'imports.txt'
    & $dumpbin /imports $Binary | Set-Content -LiteralPath $importsFile -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw "dumpbin failed: $Binary" }
    $hash = (Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash
    $size = (Get-Item -LiteralPath $Binary).Length
    & python .github/scripts/akula-win7-dumpbin-audit.py --dumpbin $importsFile --out $Output --sha256 $hash --bytes $size
    if ($LASTEXITCODE -ne 0) { throw "Win7 PE import audit failed: $Binary" }
    $result = Get-Content -LiteralPath (Join-Path $Output 'full-audit.json') -Raw | ConvertFrom-Json
    if (@($result.known_post_win7_imports).Count -ne 0) { throw 'Forbidden post-Win7 static API' }
    if ($result.binary_sha256 -cne $hash -or $result.binary_bytes -ne $size) {
        throw 'PE evidence does not match actual executable'
    }
    return $result
}

$baseAudit = Get-VerifiedPeAudit -Binary $exe -Output $dir
$comImports = @($baseAudit.imports.'combase.dll')
if ($comImports.Count -ne 2 -or $comImports -notcontains 'CoTaskMemFree' -or
    $comImports -notcontains 'CoCreateFreeThreadedMarshaler') {
    throw 'Unrecognized COMBASE dependency; only the exact audited two exports may be remapped'
}
$shivaAuditDir = Join-Path $dir 'shiva-audit'
$shivaAudit = Get-VerifiedPeAudit -Binary $shiva -Output $shivaAuditDir
if (@($shivaAudit.potentially_risky_imported_dlls) -contains 'combase.dll') {
    throw 'Win7 Shiva directly imports COMBASE'
}

$fontDir = Join-Path $env:RUNNER_TEMP 'akula-win7-signed-fonts'
New-Item -ItemType Directory -Force -Path $fontDir | Out-Null
$arial = Join-Path $env:WINDIR 'Fonts/arial.ttf'
if (!(Test-Path -LiteralPath $arial -PathType Leaf)) { throw 'System Arial required by Shiva missing' }
Copy-Item -LiteralPath $arial -Destination (Join-Path $fontDir 'arial.ttf') -Force
$env:SHIVA_FONT_DIR = $fontDir
$txt = Join-Path $dir 'pdf-smoke-input.txt'
$pdf = Join-Path $dir 'pdf-smoke-output.pdf'
[IO.File]::WriteAllText($txt, [string]::Join([Environment]::NewLine, @('Akula Win7 signed candidate PDF probe.', 'Shiva sidecar.', '')))
& $shiva $txt $pdf *> (Join-Path $dir 'pdf-smoke.log')
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $pdf -PathType Leaf)) {
    throw 'Win7 Shiva PDF converter smoke failed on modern Windows'
}
$bytes = [IO.File]::ReadAllBytes($pdf)
if ($bytes.Length -lt 64 -or [Text.Encoding]::ASCII.GetString($bytes, 0, 5) -cne '%PDF-') {
    throw 'Win7 Shiva generated an invalid PDF'
}
Remove-Item Env:\SHIVA_FONT_DIR -ErrorAction SilentlyContinue

# Fail-closed, previously lab-audited COMBASE -> OLE32 import descriptor
# remapping. Re-audit the patched bytes before NSIS packaging and signing.
& python .github/scripts/akula-win7-portable-package.py --exe $exe --audit (Join-Path $dir 'full-audit.json') --sidecar $shiva --sidecar-audit (Join-Path $shivaAuditDir 'full-audit.json') --out win7-lab-output --revision $checkout
if ($LASTEXITCODE -ne 0) { throw 'Win7 staging rejected the audited binary pair' }
$patched = Join-Path $root 'win7-lab-output/akula-app.exe'
$patchedAudit = Get-VerifiedPeAudit -Binary $patched -Output (Join-Path $root 'win7-lab-output/pe-audit')
if (@($patchedAudit.potentially_risky_imported_dlls) -contains 'combase.dll') {
    throw 'COMBASE dependency remained in signed-candidate binary'
}
$ole32 = @($patchedAudit.imports.'ole32.dll')
if ($ole32 -notcontains 'CoTaskMemFree' -or $ole32 -notcontains 'CoCreateFreeThreadedMarshaler') {
    throw 'Win7-compatible OLE32 COM imports missing'
}
Write-Host "AKULA_WIN7_SIGNED_STAGE_READY source=$checkout target=$target win7_runtime_tested=false"
