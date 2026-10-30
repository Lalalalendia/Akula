# PC-DESK-08: Windows PowerShell 2.0 compatible, real Win7 SP1 x64 test.
# Run ONLY on a disposable VM snapshot, with CI-only installer and proof.
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$Installer,
  [Parameter(Mandatory=$true)][string]$InstallerAudit,
  [Parameter(Mandatory=$true)][string]$Provenance,
  [switch]$DisposableVm,
  [switch]$UninstallAfterTest
)
$ErrorActionPreference = 'Stop'
if (-not $DisposableVm) { throw 'Use -DisposableVm on a disposable Windows 7 SP1 x64 VM snapshot' }
$os = Get-WmiObject Win32_OperatingSystem
# Server 2008 R2 SP1 shares 6.1.7601 with Windows 7 SP1: require client OS.
if ([int]$os.ProductType -ne 1) { throw 'Win7 SP1 acceptance requires a workstation Windows edition, not Windows Server' }
if ($os.Version -notmatch '^6\.1\.7601' -or [int]$os.ServicePackMajorVersion -ne 1 -or
    [string]$os.OSArchitecture -notmatch '64') {
    throw 'This test requires Windows 7 SP1 x64 build 7601'
}
foreach ($item in @($Installer, $InstallerAudit, $Provenance)) {
    if (-not [IO.File]::Exists([IO.Path]::GetFullPath($item))) { throw "Missing test file: $item" }
}
$Installer = [IO.Path]::GetFullPath($Installer)
if ([IO.Path]::GetFileName($Installer) -notmatch '^Akula_[0-9]+\.[0-9]+\.[0-9]+_x64-setup\.exe$') {
    throw 'Expected one regular Akula x64 NSIS installer'
}
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$dataDir = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$uninstallReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
foreach ($existing in @($install, $dataDir, $uninstallReg)) {
    if (Test-Path $existing) { throw "Existing Akula installation/data will not be touched: $existing" }
}
function Sha256([string]$file) {
    $stream = [IO.File]::OpenRead($file)
    try {
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $hasher.Dispose() }
    } finally { $stream.Dispose() }
}
function ProofHash([string]$body,[string]$key) {
    $m = [regex]::Match($body, '(?m)^' + [regex]::Escape($key) + '=([a-fA-F0-9]{64})\r?$')
    if (-not $m.Success) { throw "Missing expected SHA256 in CI provenance: $key" }
    return $m.Groups[1].Value.ToLowerInvariant()
}
$proof = [IO.File]::ReadAllText([IO.Path]::GetFullPath($Provenance))
$audit = [IO.File]::ReadAllText([IO.Path]::GetFullPath($InstallerAudit))
$match = [regex]::Match($audit, '"binary_sha256"\s*:\s*"([a-fA-F0-9]{64})"')
if (-not $match.Success -or $audit -notmatch '"known_post_win7_imports"\s*:\s*\[\s*\]') {
    throw 'No positive exact NSIS static PE audit'
}
if ((Sha256 $Installer) -cne $match.Groups[1].Value.ToLowerInvariant()) {
    throw 'Installer does not match the independently audited CI bytes'
}
$expectedApp = ProofHash $proof 'patched_exe_sha256'
$expectedPdf = ProofHash $proof 'shiva_pdf_sha256'
$base = Join-Path ([Environment]::GetFolderPath('MyDocuments')) ('Akula-Win7-QA-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$vault = Join-Path $base 'vault'
$privateData = Join-Path $base 'data'
$shots = Join-Path $base 'screenshots'
New-Item -ItemType Directory -Force -Path $vault, $privateData, $shots | Out-Null
$note = Join-Path $vault 'Win7 QA.md'
# Reconstruct non-ASCII fixture names in memory. UTF-8-no-BOM scripts are
# decoded as ANSI by Windows PowerShell 2/5; source code must stay ASCII.
function UnicodeLabel([int[]]$codepoints) {
    $value = ''
    foreach ($point in $codepoints) { $value += [char]$point }
    return $value
}
[IO.File]::WriteAllText($note, '# Windows 7 QA' + [Environment]::NewLine + 'Disposable note.')
# A folder chosen through Akula's ordinary picker must expose these files.
# They are created before the first GUI launch, not injected via --open.
$txt = Join-Path $vault ((UnicodeLabel @(0x041d, 0x043e, 0x0432, 0x044b, 0x0439, 0x0020, 0x0442, 0x0435, 0x043a, 0x0441, 0x0442, 0x043e, 0x0432, 0x044b, 0x0439, 0x0020, 0x0434, 0x043e, 0x043a, 0x0443, 0x043c, 0x0435, 0x043d, 0x0442)) + '.txt')
$upperTxt = Join-Path $vault ((UnicodeLabel @(0x0421, 0x0020, 0x041f, 0x0420, 0x041e, 0x0411, 0x0415, 0x041b, 0x0410, 0x041c, 0x0418)) + '.TXT')
$txtFolder = Join-Path $vault ((UnicodeLabel @(0x0412, 0x043b, 0x043e, 0x0436, 0x0435, 0x043d, 0x043d, 0x0430, 0x044f, 0x0020, 0x043f, 0x0430, 0x043f, 0x043a, 0x0430)) + '')
New-Item -ItemType Directory -Force -Path $txtFolder | Out-Null
$nestedTxt = Join-Path $txtFolder ((UnicodeLabel @(0x0412, 0x043d, 0x0443, 0x0442, 0x0440, 0x0438)) + '.txt')
$utf8 = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList @($false)
[IO.File]::WriteAllText($txt, 'AKULA_WIN7_TXT_FOLDER_NEEDLE', $utf8)
[IO.File]::WriteAllText($upperTxt, 'AKULA_WIN7_UPPERCASE_TXT', $utf8)
[IO.File]::WriteAllText($nestedTxt, 'AKULA_WIN7_NESTED_TXT', $utf8)
# Source files must be opened from the actual Win7 vault tree. This tests
# non-UTF8 formats used by Windows Notepad without modifying their bytes.
$legacyAnsi = Join-Path $vault 'Legacy-ANSI.txt'
$legacyUtf16 = Join-Path $txtFolder 'Legacy-UTF16.TXT'
$legacyBroken = Join-Path $vault 'Legacy-Corrupt.TXT'
$acp = [Text.Encoding]::Default.CodePage
$strictAnsi = [Text.Encoding]::GetEncoding($acp,
    [Text.EncoderFallback]::ExceptionFallback, [Text.DecoderFallback]::ExceptionFallback)
$strictUtf8 = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList @($false, $true)
# Find a character that this machine's real ACP can encode, but that does
# not also happen to be UTF-8. Windows 7 must exercise the ANSI fallback.
$ansiSymbol = $null
$candidates = @(
    (UnicodeLabel @(0x041f,0x0440,0x0438,0x0432,0x0435,0x0442)),
    (UnicodeLabel @(0x00e9)),
    (UnicodeLabel @(0x65e5)),
    (UnicodeLabel @(0x4e2d)),
    (UnicodeLabel @(0x03a9))
)
foreach ($candidate in $candidates) {
    try { [byte[]]$trial = $strictAnsi.GetBytes($candidate) }
    catch [Text.EncoderFallbackException] { continue }
    try { $null = $strictUtf8.GetString($trial) }
    catch [Text.DecoderFallbackException] { $ansiSymbol = $candidate; break }
}
if ($null -eq $ansiSymbol) { throw "Cannot create strict non-UTF8 TXT for Windows ANSI CP$acp" }
$ansiText = 'AKULA_WIN7_ANSI_NEEDLE ' + $ansiSymbol
[IO.File]::WriteAllBytes($legacyAnsi, $strictAnsi.GetBytes($ansiText))
$utf16Text = 'AKULA_WIN7_UTF16_NEEDLE ' + (UnicodeLabel @(0x041f,0x0440,0x0438,0x0432,0x0435,0x0442))
[byte[]]$utf16Body = [Text.Encoding]::Unicode.GetBytes($utf16Text)
$utf16Bytes = New-Object byte[] ($utf16Body.Length + 2)
$utf16Bytes[0] = 0xff
$utf16Bytes[1] = 0xfe
[Array]::Copy($utf16Body, 0, $utf16Bytes, 2, $utf16Body.Length)
[IO.File]::WriteAllBytes($legacyUtf16, $utf16Bytes)
# Bad UTF-16 with a dangling unit: must show an error, never an editable Note.
[IO.File]::WriteAllBytes($legacyBroken, [byte[]](0xff, 0xfe, 0x01))
$ansiHash = Sha256 $legacyAnsi
$utf16Hash = Sha256 $legacyUtf16
$brokenHash = Sha256 $legacyBroken
$log = Join-Path $base 'evidence.txt'
[IO.File]::WriteAllText($log, 'os=Windows 7 SP1 x64' + [Environment]::NewLine +
    'os_caption=' + $os.Caption + [Environment]::NewLine +
    'os_product_type=' + $os.ProductType + [Environment]::NewLine +
    'acceptance_script_sha256=' + (Sha256 $MyInvocation.MyCommand.Path) + [Environment]::NewLine +
    'installer_sha256=' + (Sha256 $Installer) + [Environment]::NewLine)
function Record([string]$line) { [IO.File]::AppendAllText($log, $line + [Environment]::NewLine) }
Record ('legacy_txt_ansi_codepage=' + $acp)
Record ('legacy_txt_ansi_original_sha256=' + $ansiHash)
Record ('legacy_txt_utf16_original_sha256=' + $utf16Hash)
Record ('legacy_txt_corrupt_original_sha256=' + $brokenHash)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace AkulaWin7QA {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  public static class Native {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int hgt,bool repaint);
  }
}
'@
function Snapshot([string]$file) {
    $screen = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object -TypeName System.Drawing.Bitmap -ArgumentList @($screen.Width, $screen.Height)
    $graphics = [Drawing.Graphics]::FromImage($bmp)
    try {
        $graphics.CopyFromScreen($screen.Left, $screen.Top, 0, 0, $screen.Size)
        $bmp.Save((Join-Path $shots $file), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bmp.Dispose() }
}
function OpenAkula([string]$exe) {
    $args = '--vault "' + $vault + '" --data-dir "' + $privateData + '" --open "' + $note + '" --focus'
    $proc = Start-Process -FilePath $exe -ArgumentList $args -PassThru
    for ($i=0; $i -lt 60; $i++) {
        Start-Sleep -Seconds 1
        $proc.Refresh()
        if ($proc.HasExited) { throw "Akula exited before GUI: $($proc.ExitCode)" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero) { return $proc }
    }
    throw 'No Akula window within 60 seconds on Windows 7'
}
$akula = $null
$notepad = $null
try {
    $setup = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) { throw "Akula NSIS failed: $($setup.ExitCode)" }
    $exe = Join-Path $install 'akula-app.exe'
    $pdfExe = Join-Path $install 'shiva-pdf-preview.exe'
    if (-not [IO.File]::Exists($exe) -or -not [IO.File]::Exists($pdfExe)) {
        throw 'NSIS did not install both Akula and Shiva'
    }
    if ((Sha256 $exe) -cne $expectedApp -or (Sha256 $pdfExe) -cne $expectedPdf) {
        throw 'Installed executable SHA256 mismatch against CI provenance'
    }
    Record 'installed_app_and_shiva_hashes=PASS'
    $fonts = Join-Path $base 'pdf-fonts'
    New-Item -ItemType Directory -Force -Path $fonts | Out-Null
    $arial = Join-Path $env:WINDIR 'Fonts\arial.ttf'
    if (-not [IO.File]::Exists($arial)) { throw 'Windows Arial font unavailable' }
    Copy-Item $arial (Join-Path $fonts 'arial.ttf')
    $inputTxt = Join-Path $base 'source.txt'
    $inputPdf = Join-Path $base 'roundtrip.pdf'
    $outputMd = Join-Path $base 'roundtrip.md'
    [IO.File]::WriteAllText($inputTxt, 'AKULA_WIN7_SHIVA_ROUNDTRIP_NEEDLE')
    $env:SHIVA_FONT_DIR = $fonts
    try {
        $convert = Start-Process -FilePath $pdfExe -ArgumentList ('"' + $inputTxt + '" "' + $inputPdf + '"') -Wait -PassThru
        if ($convert.ExitCode -ne 0 -or -not [IO.File]::Exists($inputPdf)) { throw 'Shiva PDF conversion failed' }
        $bytes = [IO.File]::ReadAllBytes($inputPdf)
        if ($bytes.Length -lt 64 -or [Text.Encoding]::ASCII.GetString($bytes, 0, 5) -cne '%PDF-') {
            throw 'Shiva did not create a PDF document'
        }
        $extract = Start-Process -FilePath $pdfExe -ArgumentList ('"' + $inputPdf + '" "' + $outputMd + '"') -Wait -PassThru
        if ($extract.ExitCode -ne 0 -or -not [IO.File]::Exists($outputMd) -or
            -not [IO.File]::ReadAllText($outputMd).Contains('AKULA_WIN7_SHIVA_ROUNDTRIP_NEEDLE')) {
            throw 'Shiva PDF extraction failed'
        }
    } finally { Remove-Item Env:\SHIVA_FONT_DIR -ErrorAction SilentlyContinue }
    Record 'pdf_roundtrip=PASS'
    $akula = OpenAkula $exe
    Snapshot 'initial.png'
    $rect = New-Object AkulaWin7QA.Rect
    if (-not [AkulaWin7QA.Native]::GetWindowRect($akula.MainWindowHandle, [ref]$rect)) {
        throw 'Cannot read Akula window bounds'
    }
    $overlay = Join-Path $base 'overlay.txt'
    [IO.File]::WriteAllText($overlay, 'Window overlap test.')
    $notepad = Start-Process -FilePath (Join-Path $env:WINDIR 'system32\notepad.exe') -ArgumentList ('"' + $overlay + '"') -PassThru
    for ($i=0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 300
        $notepad.Refresh()
        if ($notepad.HasExited) { throw 'Notepad overlay unexpectedly exited' }
        if ($notepad.MainWindowHandle -ne [IntPtr]::Zero) { break }
    }
    if ($notepad.MainWindowHandle -eq [IntPtr]::Zero) { throw 'No Notepad overlay window' }
    for ($i=1; $i -le 20; $i++) {
        $akula.Refresh()
        if ($akula.HasExited) { throw "Akula crashed on cover/uncover cycle $i" }
        [void][AkulaWin7QA.Native]::ShowWindow($notepad.MainWindowHandle, 9)
        [void][AkulaWin7QA.Native]::MoveWindow($notepad.MainWindowHandle, ($rect.Left+60), ($rect.Top+90), 480, 330, $true)
        [void][AkulaWin7QA.Native]::SetForegroundWindow($notepad.MainWindowHandle)
        Start-Sleep -Milliseconds 350
        if ($i -eq 1 -or $i -eq 10 -or $i -eq 20) { Snapshot ('covered-{0:D2}.png' -f $i) }
        [void][AkulaWin7QA.Native]::ShowWindow($notepad.MainWindowHandle, 0)
        [void][AkulaWin7QA.Native]::ShowWindow($akula.MainWindowHandle, 9)
        [void][AkulaWin7QA.Native]::SetForegroundWindow($akula.MainWindowHandle)
        Start-Sleep -Milliseconds 400
        Snapshot ('uncovered-{0:D2}.png' -f $i)
    }
    Record 'cover_uncover_20=COMPLETED_REQUIRES_VISUAL_REVIEW'
    $marker = 'WIN7_EDITOR_SAVE_' + [Guid]::NewGuid().ToString('N')
    Write-Host ('In Akula Win7 QA.md, type this line and SAVE from editor: ' + $marker)
    [void](Read-Host 'After saving, press Enter')
    if (-not [IO.File]::ReadAllText($note).Contains($marker)) { throw 'Editor did not persist manual Win7 edit' }
    Record 'real_editor_save=PASS'
    [void]$akula.CloseMainWindow()
    if (-not $akula.WaitForExit(15000)) { $akula.Kill(); [void]$akula.WaitForExit(10000) }
    $akula = OpenAkula $exe
    Snapshot 'after-restart.png'
    $value = Read-Host 'Read the token in the reopened Akula editor and type it here'
    if ($value -cne $marker) { throw 'Cold-reopened editor did not display the exact saved token' }
    Record 'cold_reopen=MANUALLY_CONFIRMED'
    Write-Host ('Review every uncovered PNG in ' + $shots + ' for missing/white UI regions.')
    if ((Read-Host 'Type PASS only if ALL 20 screenshots are visually correct') -cne 'PASS') {
        throw 'Win7 WM_PAINT did not pass manual visual review'
    }
    Record 'wm_paint_visual_review=PASS_MANUAL'
    # A real Win7 native GUI check. Static file enumeration on Windows 2022
    # does not establish that the Win7 UI can show or activate these entries.
    Write-Host 'In the Akula sidebar, confirm both TXT names appear, including uppercase .TXT.'
    Write-Host ('Expected: ' + [IO.Path]::GetFileName($txt) + ', ' + [IO.Path]::GetFileName($upperTxt))
    Write-Host ('Expand ' + [IO.Path]::GetFileName($txtFolder) + ' and confirm ' + [IO.Path]::GetFileName($nestedTxt))
    if ((Read-Host 'Type PASS only if all three TXT files are visible in the GUI tree') -cne 'PASS') {
        throw 'Win7 folder listing does not expose TXT files'
    }
    Record 'txt_folder_listing=PASS_MANUAL'
    Write-Host ('Click the TXT file in the sidebar: ' + [IO.Path]::GetFileName($txt))
    if ((Read-Host 'Type PASS only if Akula opens the TXT content in its editor') -cne 'PASS') {
        throw 'Win7 TXT click did not open the content'
    }
    $expectedTxtTitle = [IO.Path]::GetFileNameWithoutExtension($txt)
    $foundTitle = $false
    for ($i = 0; $i -lt 20; $i++) {
        $akula.Refresh()
        if ($akula.MainWindowTitle.Contains($expectedTxtTitle)) { $foundTitle = $true; break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $foundTitle) { throw 'Win7 TXT click did not switch active window title' }
    Record 'txt_gui_open=PASS_MANUAL_TITLE'
    if ((Read-Host 'Type PASS if the TXT body shows the exact AKULA_WIN7_TXT_FOLDER_NEEDLE') -cne 'PASS') {
        throw 'Win7 TXT content was not rendered in the editor'
    }
    Record 'txt_body=PASS_MANUAL'
    # These are manual real-Win7 GUI observations. Hashes are independently
    # checked by the script; do not equate Windows Server packaging with Win7.
    Write-Host ('Click ' + [IO.Path]::GetFileName($legacyAnsi) + ' in the Akula tree.')
    Write-Host ('Expected body: ' + $ansiText)
    Write-Host ('Expected label: Windows ANSI (' + $acp + '), read only.')
    if ((Read-Host 'Type PASS only if exact ANSI text opens without editable controls') -cne 'PASS') {
        throw 'Win7 legacy ANSI TXT failed to open read-only from vault'
    }
    Record 'legacy_txt_ansi=PASS_MANUAL'
    Write-Host ('Expand the nested folder and click ' + [IO.Path]::GetFileName($legacyUtf16))
    Write-Host ('Expected body: ' + $utf16Text)
    Write-Host 'Expected label: UTF-16 LE, read only.'
    if ((Read-Host 'Type PASS only if exact UTF-16 text opens without editable controls') -cne 'PASS') {
        throw 'Win7 legacy UTF-16 TXT failed to open read-only from nested vault'
    }
    Record 'legacy_txt_utf16=PASS_MANUAL'
    Write-Host ('Click ' + [IO.Path]::GetFileName($legacyBroken) + ' in the Akula tree.')
    if ((Read-Host 'Type PASS only if a safe open-error page appears, not an editable note') -cne 'PASS') {
        throw 'Win7 corrupt UTF-16 TXT was not rejected safely'
    }
    Record 'legacy_txt_corrupt=PASS_MANUAL'
    if ((Sha256 $legacyAnsi) -cne $ansiHash -or (Sha256 $legacyUtf16) -cne $utf16Hash -or
        (Sha256 $legacyBroken) -cne $brokenHash) {
        throw 'Opening legacy Win7 TXT changed at least one original file'
    }
    Record 'legacy_txt_bytes_unchanged=PASS'
    if ($UninstallAfterTest) {
        [void]$akula.CloseMainWindow()
        if (-not $akula.WaitForExit(15000)) { $akula.Kill(); [void]$akula.WaitForExit(10000) }
        $akula = $null
        $uninstaller = Join-Path $install 'uninstall.exe'
        if (-not [IO.File]::Exists($uninstaller)) { throw 'Akula uninstall.exe missing' }
        $remove = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
        if ($remove.ExitCode -ne 0 -or [IO.File]::Exists($exe)) { throw 'Akula uninstall failed' }
        if (-not [IO.File]::ReadAllText($note).Contains($marker)) { throw 'Uninstall changed test document' }
        Record 'nsis_uninstall=PASS_DOCUMENT_PRESERVED'
    }
    Record 'status=PASS_WIN7_SP1_USER_ACCEPTANCE'
    Write-Host ('WIN7_SP1_QA_PASS evidence=' + $base)
} finally {
    if ($null -ne $notepad) { try { if (-not $notepad.HasExited) { $notepad.Kill() } } catch {} }
    if ($null -ne $akula) { try { if (-not $akula.HasExited) { [void]$akula.CloseMainWindow() } } catch {} }
    Write-Host ('QA evidence directory: ' + $base)
}
