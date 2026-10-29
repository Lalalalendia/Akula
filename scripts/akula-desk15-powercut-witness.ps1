# DESK-15: two-phase witness for EXTERNAL power-cut experiments.
# No automated power control, installer changes, downloads, or user file removal.
# A guest cannot authenticate an external power event; approval is MANUAL ONLY.
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][ValidateSet('Prepare','Verify')][string]$Phase,
  [Parameter(Mandatory=$true)][string]$ExePath,
  [Parameter(Mandatory=$true)][string]$ExpectedSha256,
  [switch]$DisposableMachine,
  [string]$EvidenceDir,
  [string]$ExternalCutEvidence,
  [string]$TrustedCheckpointPath
)
$ErrorActionPreference = 'Stop'
if (-not $DisposableMachine) { throw 'A disposable QA workstation or VM is required' }
if ($PSVersionTable.PSVersion.Major -lt 2 -or $PSVersionTable.PSVersion.Major -gt 5) {
    throw 'Use Windows PowerShell 2 through 5, not PowerShell Core'
}
if ([Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Start Windows PowerShell with -Sta for clipboard evidence'
}
$os = Get-WmiObject Win32_OperatingSystem
if ([int]$os.ProductType -ne 1) { throw 'DESK15 power-cut witness requires a Windows workstation, not Windows Server' }
if ([string]$os.OSArchitecture -notmatch '64') { throw '64-bit workstation is required' }
$isWin7 = $os.Version -match '^6\.1\.7601' -and [int]$os.ServicePackMajorVersion -eq 1
$isModern = $os.Version -match '^10\.0\.'
if (-not $isWin7 -and -not $isModern) { throw 'Only Windows 7 SP1, 10 and 11 are accepted' }
if ($ExpectedSha256 -notmatch '^[0-9A-Fa-f]{64}$') { throw 'ExpectedSha256 must be a separately audited 64-character SHA256' }
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
$exe = [IO.Path]::GetFullPath($ExePath)
if (-not [string]::Equals($exe,[IO.Path]::GetFullPath($installed),[StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.File]::Exists($exe)) {
    throw 'Only the independently audited installed Akula may be tested'
}
function Sha256([string]$file) {
    $stream = [IO.File]::OpenRead($file)
    try {
        $h = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($h.ComputeHash($stream))).Replace('-','').ToLowerInvariant() }
        finally { $h.Dispose() }
    } finally { $stream.Dispose() }
}
function TextSha256([string]$text) {
    $h = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($h.ComputeHash($utf8.GetBytes($text)))).Replace('-','').ToLowerInvariant() }
    finally { $h.Dispose() }
}
if ((Sha256 $exe) -cne $ExpectedSha256.ToLowerInvariant()) {
    throw 'Installed EXE does not match the independently audited SHA256'
}
if (@(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'An Akula process is running: do not interfere with existing sessions'
}
$boot = [Management.ManagementDateTimeConverter]::ToDateTime($os.LastBootUpTime).ToUniversalTime().Ticks
$documents = [IO.Path]::GetFullPath([Environment]::GetFolderPath('MyDocuments'))
$utf8 = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList @($false)
Add-Type -AssemblyName System.Windows.Forms
function Require-Operator([string]$message) {
    Write-Host $message
    if ((Read-Host 'Type PASS only when independently observed') -cne 'PASS') {
        throw 'Operator did not confirm the requested GUI evidence'
    }
}
function Normalize([string]$text) {
    # Normalize CRLF/CR to LF only; do not silently discard trailing lines.
    return [regex]::Replace($text,'\r\n?',[string][char]10)
}
function Log([string]$message) {
    [IO.File]::AppendAllText($log,$message+[Environment]::NewLine,$utf8)
}
function Find-Shadow([string]$key) {
    $root = Join-Path $data 'akula-note-recovery-v1'
    if (-not (Test-Path -LiteralPath $root -PathType Container)) { return $null }
    $found = @()
    foreach ($file in @(Get-ChildItem -Path $root -Recurse -Filter '*.shadow' -ErrorAction SilentlyContinue)) {
        if ($file.PSIsContainer) { continue }
        if ([IO.File]::ReadAllText($file.FullName).Contains($key)) { $found += $file.FullName }
    }
    if ($found.Count -gt 1) { throw 'Ambiguous shadow candidates for synthetic marker' }
    if ($found.Count -eq 1) { return $found[0] }
    return $null
}
function Check-Acl([string]$shadow) {
    $root = Join-Path $data 'akula-note-recovery-v1'
    $owner = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $allowed = @($owner,'S-1-5-18','S-1-5-32-544')
    foreach ($path in @($root,(Split-Path -Parent $shadow),$shadow)) {
        $acl = Get-Acl -Path $path
        if ($null -eq $acl) { throw 'Recovery ACL missing' }
        if ($path -ne $shadow -and -not $acl.AreAccessRulesProtected) { throw 'Recovery directory has inherited ACL' }
        $actualOwner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($actualOwner -ne $owner -and $actualOwner -ne 'S-1-5-32-544') { throw 'Recovery owner mismatch' }
        $hasRead = $false
        foreach ($rule in $acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])) {
            if ($rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { continue }
            if ($allowed -notcontains $rule.IdentityReference.Value) { throw 'Broad ACL on recovery record' }
            if ($rule.IdentityReference.Value -eq $owner -and
                (($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::ReadData) -ne 0)) {
                $hasRead = $true
            }
        }
        if (-not $hasRead) { throw 'No explicit user read access to recovery record' }
    }
}
function Start-Note {
    $args = '--vault "'+$vault+'" --data-dir "'+$data+'" --open "'+$note+'" --focus'
    $proc = Start-Process -FilePath $exe -ArgumentList $args -PassThru
    for ($i=0; $i -lt 480; $i++) {
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Test-owned Akula exited unexpectedly' }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like 'PowerCut QA note*' -and
            $proc.MainWindowTitle -like '*Akula*') { return $proc }
    }
    throw 'Installed Akula did not expose the isolated note window'
}
function Wait-Shadow([Diagnostics.Process]$process) {
    for ($i=0; $i -lt 480; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited before shadow publication' }
        $found = Find-Shadow $marker
        if ($null -ne $found) { return $found }
    }
    throw 'No published shadow for dirty synthetic marker'
}
function Checked-ExternalEvidence([string]$path,[string]$trialRoot) {
    $full = [IO.Path]::GetFullPath($path)
    $prefix = $trialRoot + [IO.Path]::DirectorySeparatorChar
    if ([string]::Equals($full,$trialRoot,[StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) {
        throw 'External evidence is inside the guest trial directory'
    }
    if (-not [IO.File]::Exists($full)) { throw 'External evidence file is missing' }
    if (([IO.File]::GetAttributes($full) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'External evidence must be a regular file, not a reparse point'
    }
    # GetFullPath is lexical. Reject junctions/symlinks in ALL parents: an
    # outside-looking path must not resolve back into the disposable QA trial.
    $parent = [IO.Path]::GetDirectoryName($full)
    while (-not [string]::IsNullOrEmpty($parent)) {
        if (([IO.File]::GetAttributes($parent) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'External evidence path traverses a linked/reparse directory'
        }
        $parent = [IO.Path]::GetDirectoryName($parent)
    }
    return $full
}
function Check-TrustedCheckpoint([string]$guest,[string]$archive,[string]$trialRoot) {
    $trusted = Checked-ExternalEvidence $archive $trialRoot
    if (-not [IO.File]::Exists($guest) -or
        (([IO.File]::GetAttributes($guest) -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'Guest checkpoint is missing or a reparse point'
    }
    if ((Sha256 $guest) -cne (Sha256 $trusted)) {
        throw 'Guest checkpoint differs from independently archived host copy'
    }
    return $trusted
}
function Manifest([string]$path) {
    $values = @{}
    foreach ($line in [IO.File]::ReadAllLines($path)) {
        if ([string]::IsNullOrEmpty($line)) { continue }
        $pair = $line -split '=',2
        if ($pair.Count -ne 2 -or $values.ContainsKey($pair[0])) { throw 'Malformed or duplicate evidence field' }
        $values[$pair[0]] = $pair[1].Trim()
    }
    foreach ($name in @('protocol','trial','boot_ticks','exe_sha256','script_sha256',
                        'source_sha256','expected_sha256','shadow_sha256','marker','shadow_relative')) {
        if (-not $values.ContainsKey($name) -or [string]::IsNullOrEmpty($values[$name])) {
            throw ('Missing prepared evidence: '+$name)
        }
    }
    if ($values['protocol'] -cne 'DESK15_POWERCUT_V1') { throw 'Unknown powercut witness format' }
    return $values
}
$root = $null
if ($Phase -eq 'Prepare') {
    if ($EvidenceDir -or $ExternalCutEvidence -or $TrustedCheckpointPath) { throw 'Prepare creates its own evidence directory; do not supply Verify arguments' }
    $root = Join-Path $documents ('Akula-DESK15-POWERCUT-'+[Guid]::NewGuid().ToString('N'))
    if (Test-Path -LiteralPath $root) { throw 'Evidence path unexpectedly exists' }
    $vault = Join-Path $root 'vault'
    $data = Join-Path $root 'data'
    [void][IO.Directory]::CreateDirectory($vault)
    [void][IO.Directory]::CreateDirectory($data)
    $note = Join-Path $vault 'PowerCut QA note.md'
    [IO.File]::WriteAllText((Join-Path $data 'settings.json'),'{"editor":{"saveDebounceMs":120000}}',$utf8)
    $build = New-Object System.Text.StringBuilder
    $filler = 'x' * 185
    for ($i=0; $i -lt 100000; $i++) {
        [void]$build.Append(('Line {0:D6}: ' -f $i))
        [void]$build.Append($filler)
        [void]$build.Append(' RU=')
        [void]$build.Append([char]0x0416)
        [void]$build.Append([char]0x0438)
        [void]$build.Append([char]0x0437)
        [void]$build.Append([char]10)
    }
    $original = $build.ToString()
    $bytes = $utf8.GetByteCount($original)
    if ($bytes -le 16*1024*1024 -or $bytes -ge 24*1024*1024) { throw '100k source is outside recovery size bounds' }
    [IO.File]::WriteAllText($note,$original,$utf8)
    $marker = 'DESK15_POWERCUT_'+[Guid]::NewGuid().ToString('N')
    $expected = $original+$marker
    $expectedHash = TextSha256 $expected
    $sourceHash = Sha256 $note
    $log = Join-Path $root 'events.txt'
    Log ('trial='+[IO.Path]::GetFileName($root))
    Log ('os='+$os.Caption+' build='+$os.BuildNumber+' product='+$os.ProductType)
    Log ('source_utf8_bytes='+$bytes)
    Log ('prepared_utc='+[DateTime]::UtcNow.ToString('o'))
    $app = $null
    try {
        $app = Start-Note
        [Windows.Forms.Clipboard]::SetText($marker)
        Require-Operator 'Focus the 100k-line editor; press Ctrl+End then Ctrl+V once. Confirm ONLY the unique marker was added at EOF. Do not save/close.'
        $sentinel = 'DESK15_CLIPBOARD_'+[Guid]::NewGuid().ToString('N')
        [Windows.Forms.Clipboard]::SetText($sentinel)
        Require-Operator 'In the dirty editor press Ctrl+A and Ctrl+C. The entire 100k-line text must be copied. Do not save.'
        $readback = [Windows.Forms.Clipboard]::GetText()
        if ($readback -ceq $sentinel -or (Normalize $readback) -cne (Normalize $expected)) {
            throw 'Pre-cut GUI clipboard text does not equal exact expected Unicode draft'
        }
        if ((Sha256 $note) -cne $sourceHash) { throw 'Original was modified before the power event' }
        $shadow = Wait-Shadow $app
        Check-Acl $shadow
        $shadowHash = Sha256 $shadow
        if ((Sha256 $note) -cne $sourceHash) { throw 'Original was overwritten during shadow publication' }
        $trial = [IO.Path]::GetFileName($root)
        $shadowRel = $shadow.Substring($data.Length+1)
        $manifest = @('protocol=DESK15_POWERCUT_V1','trial='+$trial,
            'boot_ticks='+$boot,'exe_sha256='+(Sha256 $exe),
            'script_sha256='+(Sha256 $MyInvocation.MyCommand.Path),
            'source_sha256='+$sourceHash,'expected_sha256='+$expectedHash,
            'shadow_sha256='+$shadowHash,'marker='+$marker,'shadow_relative='+$shadowRel)
        [IO.File]::WriteAllLines((Join-Path $root 'checkpoint.txt'),[string[]]$manifest,$utf8)
        Log 'checkpoint=SHADOW_PUBLISHED_FILE_HASH_VERIFIED'
        Log ('shadow_sha256='+$shadowHash)
        Log ('test_process_id='+$app.Id)
        Log 'power_cut=NOT_TESTED'
        Write-Host ('DESK15_POWERCUT_PREPARED evidence='+$root)
        Write-Host 'Copy checkpoint.txt to an INDEPENDENT host before cutting power. Use ONLY an external QA power controller/hypervisor to force off this disposable test machine. Do NOT gracefully exit Akula or shut down Windows. The guest cannot verify actual power-loss.'
        $app = $null # Deliberately leave the isolated Akula process running.
    } finally {
        if ($null -ne $app) {
            try { $app.Refresh(); if (-not $app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) } }
            catch { }
        }
    }
} else {
    if ([string]::IsNullOrEmpty($EvidenceDir) -or [string]::IsNullOrEmpty($ExternalCutEvidence) -or
        [string]::IsNullOrEmpty($TrustedCheckpointPath)) {
        throw 'Verify requires -EvidenceDir, -ExternalCutEvidence and -TrustedCheckpointPath'
    }
    $root = [IO.Path]::GetFullPath($EvidenceDir)
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($root),$documents,
                             [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($root).StartsWith('Akula-DESK15-POWERCUT-')) {
        throw 'Refuse to verify outside a direct isolated Documents QA directory'
    }
    if (-not [IO.Directory]::Exists($root) -or
        (([IO.File]::GetAttributes($root) -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'Missing or linked evidence directory'
    }
    $vault = Join-Path $root 'vault'
    $data = Join-Path $root 'data'
    $note = Join-Path $vault 'PowerCut QA note.md'
    $log = Join-Path $root 'events.txt'
    $guestCheckpoint = Join-Path $root 'checkpoint.txt'
    $trustedCheckpoint = Check-TrustedCheckpoint $guestCheckpoint $TrustedCheckpointPath $root
    $m = Manifest $guestCheckpoint
    Log ('trusted_checkpoint_sha256='+(Sha256 $trustedCheckpoint))
    Log 'trusted_checkpoint_copy=MATCHES_HOST_ARCHIVE_NOT_INDEPENDENTLY_AUTHENTICATED'
    if ($m['trial'] -cne [IO.Path]::GetFileName($root) -or
        $m['exe_sha256'] -cne (Sha256 $exe) -or
        $m['script_sha256'] -cne (Sha256 $MyInvocation.MyCommand.Path)) {
        throw 'Trial, installed application, or script checksum changed between phases'
    }
    if ($m['boot_ticks'] -ceq [string]$boot) { throw 'No reboot observed since preparation' }
    if ((Sha256 $note) -cne $m['source_sha256']) { throw 'Original changed across power event before Restore' }
    $marker = $m['marker']
    $shadow = Find-Shadow $marker
    if ($null -eq $shadow) { throw 'Prepared shadow missing after reboot' }
    if ($shadow.Substring($data.Length+1) -cne $m['shadow_relative'] -or
        (Sha256 $shadow) -cne $m['shadow_sha256']) {
        throw 'Published recovery shadow bytes changed after reboot'
    }
    Check-Acl $shadow
    $proofPath = Checked-ExternalEvidence $ExternalCutEvidence $root
    if ([string]::Equals($proofPath,$trustedCheckpoint,[StringComparison]::OrdinalIgnoreCase)) {
        throw 'Power event proof and archived checkpoint must be separate files'
    }
    $proof = [IO.File]::ReadAllText($proofPath)
    if (-not $proof.Contains($m['trial']) -or
        $proof -notmatch '(?m)^POWER_EVENT=(FORCED_VM_OFF|PHYSICAL_POWER_LOSS)\r?$') {
        throw 'Host event record must identify trial and explicit forced-off event type'
    }
    Log ('external_event_proof_file_sha256='+(Sha256 $proofPath))
    Log 'external_event_proof=PROVIDED_FOR_MANUAL_AUDIT_NOT_INDEPENDENTLY_AUTHENTICATED'
    Log ('post_reboot_boot_ticks='+$boot)
    $app = $null
    try {
        $app = Start-Note
        if ((Sha256 $note) -cne $m['source_sha256']) { throw 'Application overwrote source before Restore consent' }
        Log 'no_auto_overwrite=PASS'
        Require-Operator 'Confirm Restore draft is visibly offered for PowerCut QA note. Do not click before typing PASS.'
        if ((Sha256 $note) -cne $m['source_sha256']) { throw 'Source changed before Restore consent' }
        Require-Operator 'Click Restore draft once. Confirm the dialog closes.'
        $saved = $false
        for ($i=0; $i -lt 480; $i++) {
            Start-Sleep -Milliseconds 250
            if ((Sha256 $note) -ceq $m['expected_sha256'] -and
                $null -eq (Find-Shadow $marker)) { $saved = $true; break }
        }
        if (-not $saved) { throw 'Restore file hash or verified shadow retirement did not pass' }
        $sentinel = 'DESK15_VERIFY_CLIPBOARD_'+[Guid]::NewGuid().ToString('N')
        [Windows.Forms.Clipboard]::SetText($sentinel)
        Require-Operator 'Focus restored editor; press Ctrl+A and Ctrl+C. Return only after the full document is copied.'
        $clipboard = [Windows.Forms.Clipboard]::GetText()
        if ($clipboard -ceq $sentinel -or
            (TextSha256 (Normalize $clipboard)) -cne (TextSha256 (Normalize ([IO.File]::ReadAllText($note))))) {
            throw 'Post-reboot GUI readback differs from recovered source'
        }
        Log 'source_unchanged_before_restore=PASS'
        Log ('recovered_sha256='+(Sha256 $note))
        Log 'full_gui_readback=PASS'
        Log 'shadow_retired=PASS'
        Log 'status=RESTORE_PASS_EXTERNAL_POWER_EVENT_REQUIRES_MANUAL_AUDIT'
        Write-Host ('DESK15_RESTORE_VERIFIED_EXTERNAL_EVENT_NOT_AUTHENTICATED evidence='+$root)
    } catch {
        Log ('status=FAIL_'+$_.Exception.GetType().Name)
        throw
    } finally {
        if ($null -ne $app) {
            try { $app.Refresh(); if (-not $app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) } }
            catch { }
        }
        Write-Host ('Retained isolated QA evidence and synthetic source at: '+$root)
    }
}
