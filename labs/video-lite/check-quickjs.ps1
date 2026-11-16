# Non-release experiment: replace Deno with tiny, pinned QuickJS-NG.
# No YouTube URL is ever passed. No file is staged into an Akula vault.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$ScratchRoot)
$ErrorActionPreference = 'Stop'
$version = 'v0.18.0'
$qjsSha256 = 'ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7'
$ytdlpVersion = '2026.10.26'
$ytdlpSha256 = 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'

function Assert-Hash([string]$Path, [string]$Expected) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'VIDEO05_MISSING_BINARY' }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $Expected) { throw "VIDEO05_HASH_REJECTED expected=$Expected actual=$actual" }
}
$dir = Join-Path $ScratchRoot ('video05-rust-quickjs-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($dir)
try {
    $tampered = Join-Path $dir 'tampered.exe'
    [IO.File]::WriteAllText($tampered, 'untrusted data')
    $rejected = $false
    try { Assert-Hash $tampered $qjsSha256 }
    catch { $rejected = $_.Exception.Message.Contains('VIDEO05_HASH_REJECTED') }
    if (!$rejected) { throw 'VIDEO05_FAILED_TO_REJECT_TAMPERED_QJS' }

    $qjs = Join-Path $dir 'qjs.exe'
    $ytdlp = Join-Path $dir 'yt-dlp.exe'
    Invoke-WebRequest "https://github.com/quickjs-ng/quickjs/releases/download/$version/qjs-windows-x86_64.exe" -OutFile $qjs -UseBasicParsing -MaximumRedirection 8
    Assert-Hash $qjs $qjsSha256
    Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/download/$ytdlpVersion/yt-dlp.exe" -OutFile $ytdlp -UseBasicParsing -MaximumRedirection 8
    Assert-Hash $ytdlp $ytdlpSha256

    $eval = (& $qjs -e 'print("VIDEO05_QJS_EVAL_PASS")' 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or !$eval.Contains('VIDEO05_QJS_EVAL_PASS')) {
        throw "VIDEO05_QJS_EVAL_FAILED: $eval"
    }
    # This validates CLI selection, not the live YouTube JS challenge.
    $cli = (& $ytdlp --no-js-runtimes --js-runtimes "quickjs:$qjs" --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $cli -cne $ytdlpVersion) {
        throw "VIDEO05_YTDLP_QJS_ARGS_FAILED: $cli"
    }

    $directBytes = (Get-Item -LiteralPath $qjs).Length + (Get-Item -LiteralPath $ytdlp).Length
    $bundle = Join-Path $dir 'quickjs-and-ytdlp.zip'
    Compress-Archive -LiteralPath $qjs,$ytdlp -DestinationPath $bundle -CompressionLevel Optimal
    $compressedBytes = (Get-Item -LiteralPath $bundle).Length
    Write-Host "VIDEO05_QJS_PIN_PASS sha256=$qjsSha256 quickjs_bytes=$((Get-Item $qjs).Length)"
    Write-Host "VIDEO05_YTDLP_PIN_PASS sha256=$ytdlpSha256 ytdlp_bytes=$((Get-Item $ytdlp).Length)"
    Write-Host "VIDEO05_SLIM_SIZES compressed_bytes=$compressedBytes executables_bytes=$directBytes"
    Write-Host 'VIDEO05_JS_NONRELEASE_PASS quickjs_exec=1 yt_dlp_args=1 youtube_challenge=NOT_TESTED'
} finally {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
