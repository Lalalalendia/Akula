# VIDEO-04: reproducible Windows 10+ *non-release* FFmpeg/Deno/yt-dlp lab.
# No media/network extractor requests; no downloads into any Akula vault or release.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ScratchRoot,
    [string]$FixturePath = 'testdata/video-toolchain/avc_aac_2s.mp4'
)
$ErrorActionPreference = 'Stop'

$ffmpegRelease = 'autobuild-2026-11-15-12-40'
$ffmpegAsset = 'ffmpeg-n8.1.3-35-gf1acb0a266-win64-lgpl-8.1.zip'
$ffmpegSha256 = '272140baa96cbb9553eb57c03590e0b8d2c93ce352aa7f876c34fbc657536836'
$denoVersion = 'v2.9.8'
$denoAsset = 'deno-x86_64-pc-windows-msvc.zip'
$denoSha256 = 'cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf'
$ytVersion = '2026.10.26'
$ytSha256 = 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'

function Assert-PinnedHash([string]$Path, [string]$Expected) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) { throw "VIDEO04_MISSING_FILE: $Path" }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $Expected.ToLowerInvariant()) {
        throw "VIDEO04_BAD_SHA256: expected=$Expected actual=$actual"
    }
}
function Fetch-Pinned([string]$Url, [string]$Path, [string]$Expected) {
    Invoke-WebRequest -Uri $Url -OutFile $Path -MaximumRedirection 8 -UseBasicParsing -TimeoutSec 240
    Assert-PinnedHash $Path $Expected
}
function Only-OneExe([string]$Root, [string]$Name) {
    $items = @(Get-ChildItem -LiteralPath $Root -File -Recurse -Filter $Name)
    if ($items.Count -ne 1) { throw "VIDEO04_AMBIGUOUS_TOOL: $Name count=$($items.Count)" }
    return $items[0].FullName
}
function Check-Exit([string]$Step) {
    if ($LASTEXITCODE -ne 0) { throw "VIDEO04_TOOL_FAILED: $Step exit=$LASTEXITCODE" }
}

$fixture = (Resolve-Path -LiteralPath $FixturePath -ErrorAction Stop).Path
$fixtureSha = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
$trial = Join-Path $ScratchRoot ('akula-video04-ffmpeg-deno-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $trial -Force | Out-Null
try {
    # Fail closed before downloading/executing any third-party code.
    $bad = Join-Path $trial 'tampered.zip'
    [IO.File]::WriteAllText($bad, 'untrusted archive')
    $rejected = $false
    try { Assert-PinnedHash $bad $ffmpegSha256 }
    catch { $rejected = $_.Exception.Message.Contains('VIDEO04_BAD_SHA256') }
    if (!$rejected) { throw 'VIDEO04_NEGATIVE_SHA_TEST_FAILED' }
    Write-Host 'VIDEO04_FFMPEG_NEGATIVE_SHA_PASS no_execution=1'

    $ffZip = Join-Path $trial $ffmpegAsset
    $denoZip = Join-Path $trial $denoAsset
    $ytExe = Join-Path $trial 'yt-dlp.exe'
    Fetch-Pinned "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/$ffmpegRelease/$ffmpegAsset" $ffZip $ffmpegSha256
    Fetch-Pinned "https://github.com/denoland/deno/releases/download/$denoVersion/$denoAsset" $denoZip $denoSha256
    Fetch-Pinned "https://github.com/yt-dlp/yt-dlp/releases/download/$ytVersion/yt-dlp.exe" $ytExe $ytSha256
    Write-Host 'VIDEO04_ALL_DOWNLOAD_SHA256_PASS archives_verified_before_extract=1'

    $ffDir = Join-Path $trial 'ffmpeg'
    $denoDir = Join-Path $trial 'deno'
    Expand-Archive -LiteralPath $ffZip -DestinationPath $ffDir
    Expand-Archive -LiteralPath $denoZip -DestinationPath $denoDir
    $ffmpeg = Only-OneExe $ffDir 'ffmpeg.exe'
    $ffprobe = Only-OneExe $ffDir 'ffprobe.exe'
    $deno = Only-OneExe $denoDir 'deno.exe'

    $ffVersion = (& $ffmpeg -hide_banner -version 2>&1 | Out-String)
    Check-Exit 'ffmpeg -version'
    if (!$ffVersion.Contains('n8.1.3') -or $ffVersion.Contains('--enable-gpl') -or $ffVersion.Contains('--enable-nonfree')) {
        throw 'VIDEO04_FFMPEG_VERSION_OR_LICENSE_MISMATCH'
    }
    $ffLicense = (& $ffmpeg -L 2>&1 | Out-String)
    Check-Exit 'ffmpeg -L'
    if (!$ffLicense.Contains('Lesser General Public License')) {
        throw 'VIDEO04_FFMPEG_NOT_LGPL'
    }
    $denoOutput = (& $deno --version 2>&1 | Out-String)
    Check-Exit 'deno --version'
    if (!$denoOutput.Contains('deno 2.9.8')) { throw "VIDEO04_DENO_VERSION_MISMATCH: $denoOutput" }
    $evalOutput = (& $deno eval --no-config 'console.log("AKULA_DENO_EVAL_PASS")' 2>&1 | Out-String)
    Check-Exit 'deno eval'
    if (!$evalOutput.Contains('AKULA_DENO_EVAL_PASS')) { throw 'VIDEO04_DENO_JS_EXECUTION_FAILED' }
    $ytOutput = (& $ytExe --js-runtimes "deno:$deno" --version 2>&1 | Out-String)
    Check-Exit 'yt-dlp runtime argument parser'
    if ($ytOutput.Trim() -cne $ytVersion) { throw 'VIDEO04_YTDLP_VERSION_MISMATCH' }
    Write-Host 'VIDEO04_DENO_RUNTIME_PASS js_eval=1 yt_dlp_parser=1 youtube_challenge=NOT_TESTED'

    # Real H.264/AAC bytes, from VIDEO-03's original 2-second black/quiet clip.
    # Split into elementary logical tracks, then remux with stream-copy only.
    # No GPU, re-encoding, real YouTube request or test user's personal data.
    $videoOnly = Join-Path $trial 'video-only.mp4'
    $audioOnly = Join-Path $trial 'audio-only.m4a'
    $merged = Join-Path $trial 'remuxed.mp4'
    & $ffmpeg -hide_banner -loglevel error -nostdin -y -i $fixture -map '0:v:0' -c:v copy -an $videoOnly
    Check-Exit 'ffmpeg isolate H264'
    & $ffmpeg -hide_banner -loglevel error -nostdin -y -i $fixture -map '0:a:0' -c:a copy -vn $audioOnly
    Check-Exit 'ffmpeg isolate AAC'
    & $ffmpeg -hide_banner -loglevel error -nostdin -y -i $videoOnly -i $audioOnly -map '0:v:0' -map '1:a:0' -c copy -movflags +faststart $merged
    Check-Exit 'ffmpeg remux H264 + AAC'
    if ((Get-Item -LiteralPath $merged).Length -le 1024) { throw 'VIDEO04_REMUX_EMPTY' }
    $streamsJson = & $ffprobe -v error -show_entries stream=codec_type,codec_name -of json $merged
    Check-Exit 'ffprobe codecs'
    $streams = @((($streamsJson | Out-String) | ConvertFrom-Json).streams)
    if ($streams.Count -ne 2) { throw "VIDEO04_BAD_STREAM_COUNT: $($streams.Count)" }
    $videoOk = @($streams | Where-Object { $_.codec_type -eq 'video' -and $_.codec_name -eq 'h264' }).Count -eq 1
    $audioOk = @($streams | Where-Object { $_.codec_type -eq 'audio' -and $_.codec_name -eq 'aac' }).Count -eq 1
    if (!$videoOk -or !$audioOk) { throw 'VIDEO04_REMUX_CHANGED_CODECS' }
    & $ffmpeg -hide_banner -loglevel error -nostdin -i $merged -map '0:v:0' -f null NUL
    Check-Exit 'ffmpeg decode remuxed H264'
    Write-Host "VIDEO04_FFMPEG_REMUX_PASS output_sha256=$((Get-FileHash $merged -Algorithm SHA256).Hash.ToLowerInvariant()) h264=1 aac=1 no_reencode=1"

    $sumArchives = (Get-Item -LiteralPath $ffZip).Length + (Get-Item -LiteralPath $denoZip).Length + (Get-Item -LiteralPath $ytExe).Length
    $sumExecutables = (Get-Item -LiteralPath $ffmpeg).Length + (Get-Item -LiteralPath $ffprobe).Length + (Get-Item -LiteralPath $deno).Length + (Get-Item -LiteralPath $ytExe).Length
    Write-Host "VIDEO04_SIZE_REPORT compressed_bytes=$sumArchives core_executables_bytes=$sumExecutables"
    Write-Host "VIDEO04_TEST_FIXTURE_SHA256=$fixtureSha"
    Write-Host 'VIDEO04_NONRELEASE_PASS no_yt_video_request=1 no_installer=1 no_cloud_sync=1'
    Write-Host 'VIDEO04_NOT_ACCEPTED win7=unchecked gui=unchecked ytdlp_real_youtube=unchecked licenses=needs_review'
} finally {
    Remove-Item -LiteralPath $trial -Recurse -Force -ErrorAction SilentlyContinue
}
