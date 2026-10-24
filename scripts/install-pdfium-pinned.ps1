# AKULA-PDF-VIEW-01: deterministic PDFium resource for the Windows beta.
# Archive is pinned by sha256. Avoid enabling PDF JavaScript/V8.
param(
  [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$tag = 'chromium/8111'
$archiveSha256 = '3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e'
$url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$tag/pdfium-win-x64.tgz"
$stage = Join-Path $env:RUNNER_TEMP ('akula-pdfium-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $stage | Out-Null
try {
  $archive = Join-Path $stage 'pdfium-win-x64.tgz'
  Invoke-WebRequest -Uri $url -OutFile $archive -MaximumRedirection 10
  $observed = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($observed -cne $archiveSha256) {
    throw "Pinned PDFium archive SHA-256 mismatch: $observed"
  }
  # Extract just the expected known payload; never extract arbitrary archive paths.
  & tar -xzf $archive -C $stage 'bin/pdfium.dll'
  if ($LASTEXITCODE -ne 0) { throw 'Pinned PDFium archive extraction failed' }
  $source = Join-Path $stage 'bin/pdfium.dll'
  if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
    throw 'Pinned PDFium archive lacks bin/pdfium.dll'
  }
  $bytes = [IO.File]::ReadAllBytes($source)
  if ($bytes.Length -lt 1024 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    throw 'Pinned PDFium DLL is not a PE image'
  }
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  if ($pe -lt 0x40 -or $pe -gt ($bytes.Length - 8) -or
      $bytes[$pe] -ne 0x50 -or $bytes[$pe + 1] -ne 0x45 -or
      $bytes[$pe + 2] -ne 0 -or $bytes[$pe + 3] -ne 0) {
    throw 'Pinned PDFium DLL has an invalid PE header'
  }
  if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) {
    throw 'Pinned PDFium DLL is not x86-64'
  }
  New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
  $destination = Join-Path $OutputDirectory 'pdfium.dll'
  Copy-Item -LiteralPath $source -Destination $destination -Force
  $dllHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
  Write-Host "AKULA_PDFIUM_PINNED=PASS;tag=$tag;archive_sha256=$archiveSha256;dll_sha256=$dllHash;size=$($bytes.Length)"
} finally {
  Remove-Item -LiteralPath $stage -Force -Recurse -ErrorAction SilentlyContinue
}
