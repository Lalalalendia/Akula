# DESK-15: manual, real-OS hard-kill recovery acceptance for Windows 7 SP1 / 10 / 11.
# Windows PowerShell 2.0-compatible (run powershell.exe -Sta -File ...).
# ONLY on a disposable, isolated QA machine/VM with an independently audited
# Akula installation. This script never downloads, installs, uninstalls, deletes
# user files, or power-cycles a machine. It kills ONLY its own test process.
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$ExePath,
  [Parameter(Mandatory=$true)][string]$ExpectedSha256,
  [switch]$DisposableMachine,
  [switch]$LargeDocument
)
$ErrorActionPreference = 'Stop'
if (-not $DisposableMachine) { throw 'A disposable test machine or VM is required' }
if ($PSVersionTable.PSVersion.Major -lt 2 -or $PSVersionTable.PSVersion.Major -gt 5) { throw 'Use Windows PowerShell 2 through 5, not PowerShell Core' }
if ([Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Start Windows PowerShell with -Sta to allow foreground clipboard proof'
}
$os = Get-WmiObject Win32_OperatingSystem
# Windows Server 2022 also reports 10.0; Win7 and Server 2008 R2 share 6.1.7601.
# WMI ProductType=1 is the supported workstation edition, 2/3 are servers.
if ([int]$os.ProductType -ne 1) { throw 'DESK-15 acceptance requires a workstation Windows edition, not Windows Server' }
if ([string]$os.OSArchitecture -notmatch '64') { throw 'DESK-15 real-OS test requires x64' }
$isWin7 = $os.Version -match '^6\.1\.7601' -and [int]$os.ServicePackMajorVersion -eq 1
$isWinModern = $os.Version -match '^10\.0\.'
if (-not $isWin7 -and -not $isWinModern) { throw 'Test supports only Windows 7 SP1, Windows 10 or Windows 11' }
if ($ExpectedSha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'ExpectedSha256 must be a 64-character audited digest' }
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
$exe = [IO.Path]::GetFullPath($ExePath)
if (-not [string]::Equals($exe, [IO.Path]::GetFullPath($installed),
    [StringComparison]::OrdinalIgnoreCase) -or -not [IO.File]::Exists($exe)) {
    throw 'Only the ordinary installed Akula under LocalAppData may be tested'
}
function Sha256([string]$file) {
    $stream = [IO.File]::OpenRead($file)
    try {
        $hash = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($hash.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $hash.Dispose() }
    } finally { $stream.Dispose() }
}
if ((Sha256 $exe) -cne $ExpectedSha256.ToLowerInvariant()) {
    throw 'Installed Akula does not match the independently audited expected SHA256'
}
if (@(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'An Akula process is already running; refuse to interfere with it'
}
Add-Type -AssemblyName System.Windows.Forms
$base = Join-Path ([Environment]::GetFolderPath('MyDocuments')) ('Akula-DESK15-QA-' + [Guid]::NewGuid().ToString('N'))
if (Test-Path $base) { throw 'Evidence directory unexpectedly exists' }
$vault = Join-Path $base 'vault'
$data = Join-Path $base 'data'
[void][IO.Directory]::CreateDirectory($vault)
[void][IO.Directory]::CreateDirectory($data)
$utf8 = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList @($false)
$first = Join-Path $vault 'First note.md'
$second = Join-Path $vault 'Second note.md'
$initialA = '# First note' + [Environment]::NewLine + 'Unmodified first source'
$initialB = '# Second note' + [Environment]::NewLine + 'Unmodified second source'
if ($LargeDocument) {
    # 100,000 Unicode lines, 16-24 MiB in UTF-8; ASCII-only source for PS2.
    # The large baseline is already on disk: type only a small dirty marker.
    $large = New-Object System.Text.StringBuilder
    $filler = 'x' * 185
    for ($i = 0; $i -lt 100000; $i++) {
        [void]$large.Append(('Line {0:D6}: ' -f $i))
        [void]$large.Append($filler)
        [void]$large.Append(' RU=')
        [void]$large.Append([char]0x0416)
        [void]$large.Append([char]0x0438)
        [void]$large.Append([char]0x0437)
        [void]$large.Append([char]10)
    }
    $initialB = $large.ToString()
    $size = $utf8.GetByteCount($initialB)
    if ($size -le 16 * 1024 * 1024 -or $size -gt 24 * 1024 * 1024) {
        throw '100k-line QA fixture is outside the 16-24 MiB shadow boundary'
    }
}
[IO.File]::WriteAllText($first, $initialA, $utf8)
[IO.File]::WriteAllText($second, $initialB, $utf8)
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),
    '{"editor":{"saveDebounceMs":120000}}', $utf8)
$sourceA = Sha256 $first
$sourceB = Sha256 $second
$draftA = '# First note' + [char]10 + 'DESK15_QA_A_' + [Guid]::NewGuid().ToString('N')
$markerB = 'DESK15_QA_B_' + [Guid]::NewGuid().ToString('N')
if ($LargeDocument) { $draftB = $initialB + $markerB }
else { $draftB = '# Second note' + [char]10 + $markerB }
$markerA = ($draftA -split [char]10)[1]
$shadowBase = Join-Path $data 'akula-note-recovery-v1'
$log = Join-Path $base 'evidence.txt'
[IO.File]::WriteAllText($log, ('os_version=' + $os.Version + [Environment]::NewLine +
    'os_build=' + $os.BuildNumber + [Environment]::NewLine +
    'os_caption=' + $os.Caption + [Environment]::NewLine +
    'os_product_type=' + $os.ProductType + [Environment]::NewLine +
    'acceptance_script_sha256=' + (Sha256 $MyInvocation.MyCommand.Path) + [Environment]::NewLine +
    'installed_exe_sha256=' + (Sha256 $exe) + [Environment]::NewLine +
    'first_original_sha256=' + $sourceA + [Environment]::NewLine +
    'second_original_sha256=' + $sourceB + [Environment]::NewLine), $utf8)
if ($LargeDocument) {
    [IO.File]::AppendAllText($log,
        ('large_fixture_lines=100000' + [Environment]::NewLine +
         'large_fixture_utf8_bytes=' + $utf8.GetByteCount($initialB) + [Environment]::NewLine +
         'large_expected_draft_sha256=' +
         ([BitConverter]::ToString(([Security.Cryptography.SHA256]::Create()).ComputeHash($utf8.GetBytes($draftB)))).Replace('-', '').ToLowerInvariant() +
         [Environment]::NewLine))
}
function Record([string]$line) { [IO.File]::AppendAllText($log, $line + [Environment]::NewLine) }
function Normalized([string]$value) {
    return ([regex]::Replace($value, '\r\n?', [string][char]10)).TrimEnd([char]10)
}
function Prompt-Pass([string]$instruction) {
    Write-Host $instruction
    if ((Read-Host 'Type PASS only when the stated GUI observation is true') -cne 'PASS') {
        throw 'Operator did not confirm the required real-OS GUI observation'
    }
}
function Start-Fixture([string]$path, [string]$title) {
    $args = '--vault "' + $vault + '" --data-dir "' + $data + '" --open "' + $path + '" --focus'
    $process = Start-Process -FilePath $exe -ArgumentList $args -PassThru
    $pollLimit = 120
    if ($LargeDocument) { $pollLimit = 480 }
    for ($i=0; $i -lt $pollLimit; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Installed Akula exited before opening the fixture' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and
            $process.MainWindowTitle -like ($title + '*') -and
            $process.MainWindowTitle -like '*Akula*') { return $process }
    }
    throw 'Installed Akula did not expose the expected note window'
}
function Find-Shadow([string]$marker) {
    if (-not (Test-Path $shadowBase)) { return $null }
    $found = @()
    foreach ($item in @(Get-ChildItem -Path $shadowBase -Filter '*.shadow' -Recurse -ErrorAction SilentlyContinue)) {
        if ($item.PSIsContainer) { continue }
        try {
            if ([IO.File]::ReadAllText($item.FullName).Contains($marker)) { $found += $item.FullName }
        } catch {
            # An in-flight atomic rename or a pending writer is not yet proof.
        }
    }
    if ($found.Count -gt 1) { throw 'Ambiguous recovery snapshot for one synthetic draft' }
    if ($found.Count -eq 1) { return $found[0] }
    return $null
}
function Wait-Shadow([string]$marker, [Diagnostics.Process]$process) {
    for ($i=0; $i -lt 160; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited before durable snapshot publication' }
        $file = Find-Shadow $marker
        if ($null -ne $file) { return $file }
    }
    throw 'Complete synthetic draft did not appear in a durable recovery snapshot'
}
function Check-Acl([string]$snapshot) {
    $current = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $allowed = @($current, 'S-1-5-18', 'S-1-5-32-544')
    foreach ($path in @($shadowBase, (Split-Path -Parent $snapshot), $snapshot)) {
        $acl = Get-Acl -Path $path
        if ($null -eq $acl) { throw 'Missing recovery ACL' }
        if ($path -ne $snapshot -and -not $acl.AreAccessRulesProtected) {
            throw 'Recovery directory inherits potentially unsafe permissions'
        }
        $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($owner -ne $current -and $owner -ne 'S-1-5-32-544') {
            throw 'Recovery snapshot owner is not the QA user or Administrators'
        }
        $hasRead = $false
        foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
            if ($rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { continue }
            if ($allowed -notcontains $rule.IdentityReference.Value) { throw 'Broad recovery ACL access' }
            if ($rule.IdentityReference.Value -eq $current -and
                (($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::ReadData) -ne 0)) {
                $hasRead = $true
            }
        }
        if (-not $hasRead) { throw 'No explicit user read access to recovery snapshot' }
    }
}
$app = $null
$passed = $false
try {
    $app = Start-Fixture $first 'First note'
    [Windows.Forms.Clipboard]::SetText($draftA)
    Prompt-Pass 'In the FIRST note editor press Ctrl+A, then Ctrl+V (synthetic clipboard draft A). Do NOT save. Check draft A is visible.'
    if ((Sha256 $first) -cne $sourceA) { throw 'First source changed before detached save' }
    $shadowA = Wait-Shadow $markerA $app
    Check-Acl $shadowA
    Record 'first_staged_shadow_and_acl=PASS'
    Prompt-Pass 'Hold Ctrl+Shift and click SECOND note in the file tree. Both tabs must remain visible, with Second note active.'
    $app.Refresh()
    if ($app.MainWindowTitle -notlike 'Second note*') { throw 'Ctrl+Shift+click did not activate Second note' }
    $firstSaved = $false
    for ($i=0; $i -lt 240; $i++) {
        Start-Sleep -Milliseconds 250
        if ((Normalized ([IO.File]::ReadAllText($first))) -ceq (Normalized $draftA) -and
            $null -eq (Find-Shadow $markerA)) { $firstSaved = $true; break }
    }
    if (-not $firstSaved) { throw 'Detached first tab did not save and retire shadow on File Gate ACK' }
    $firstSavedHash = Sha256 $first
    Record ('first_detached_ack=PASS sha256=' + $firstSavedHash)
    if ($LargeDocument) {
        [Windows.Forms.Clipboard]::SetText($markerB)
        Prompt-Pass 'Wait for the 100,000-line SECOND note to load fully. Focus editor, press Ctrl+End, then Ctrl+V (small dirty marker). Do NOT save. Check marker is visible.'
        Record 'large_unicode_dirty_marker_inserted_by_operator=PASS'
    } else {
        [Windows.Forms.Clipboard]::SetText($draftB)
        Prompt-Pass 'In the SECOND note editor press Ctrl+A, then Ctrl+V (synthetic clipboard draft B). Do NOT save.'
    }
    if ((Sha256 $second) -cne $sourceB) { throw 'Second source changed before crash' }
    $shadowB = Wait-Shadow $markerB $app
    Check-Acl $shadowB
    Record 'second_staged_shadow_and_acl=PASS'
    $app.Refresh()
    if ($app.MainWindowTitle -notlike 'Second note*') { throw 'Expected second document not active before kill' }
    $app.Kill()
    if (-not $app.WaitForExit(10000)) { throw 'Disposable Akula process did not exit after hard kill' }
    $app = $null
    Record 'owned_akula_process_hard_killed=PASS'
    if ((Sha256 $second) -cne $sourceB) { throw 'Second source changed during crash' }
    $app = Start-Fixture $second 'Second note'
    Start-Sleep -Seconds 2
    if ((Sha256 $second) -cne $sourceB) { throw 'Akula automatically overwrote second source on restart' }
    Record 'no_auto_overwrite=PASS'
    Prompt-Pass 'Confirm Akula visibly offers Restore draft for SECOND note. Do NOT click Restore until after typing PASS here.'
    if ((Sha256 $second) -cne $sourceB) { throw 'Recovery UI appeared after an automatic file overwrite' }
    Prompt-Pass 'Now click the visible Restore draft button for SECOND note. Confirm it closes.'
    $restored = $false
    for ($i=0; $i -lt 160; $i++) {
        Start-Sleep -Milliseconds 250
        if ((Normalized ([IO.File]::ReadAllText($second))) -ceq (Normalized $draftB) -and
            $null -eq (Find-Shadow $markerB)) { $restored = $true; break }
    }
    if (-not $restored) { throw 'File Gate restore or verified shadow retirement did not complete' }
    if ((Sha256 $first) -cne $firstSavedHash) { throw 'Recovery changed the previously saved first tab' }
    Record ('second_restored_and_retired=PASS sha256=' + (Sha256 $second))
    $sentinel = 'DESK15_QA_CLIPBOARD_SENTINEL_' + [Guid]::NewGuid().ToString('N')
    [Windows.Forms.Clipboard]::SetText($sentinel)
    Prompt-Pass 'In the restored SECOND note, focus body then press Ctrl+A and Ctrl+C. Return here only after the copy.'
    $readback = [Windows.Forms.Clipboard]::GetText()
    if ($readback -ceq $sentinel -or
        (Normalized $readback) -cne (Normalized ([IO.File]::ReadAllText($second)))) {
        throw 'Real GUI clipboard readback differs from the exact recovered file'
    }
    Record 'restored_second_gui_exact=PASS'
    if ($LargeDocument) { Record 'large_100k_unicode_gui_and_file_sha256_exact=PASS' }
    Record 'power_cut=NOT_TESTED'
    Record 'status=PASS_DESK15_REAL_OS_MANUAL_HARD_KILL'
    $passed = $true
    Write-Host ('DESK15_REAL_OS_MANUAL_PASS evidence=' + $base)
} catch {
    Record ('status=FAIL_' + $_.Exception.GetType().Name)
    throw
} finally {
    if ($null -ne $app) {
        try { $app.Refresh(); if (-not $app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) } }
        catch { }
    }
    Write-Host ('DESK-15 isolated evidence and both synthetic notes retained at: ' + $base)
}
