# DESK-13/15: hosted installed 100k Unicode-line crash/Restore acceptance.
# Synthetic hosted Windows CI only; never execute on a developer workstation.
param([Parameter(Mandatory=$true)][string]$ExePath)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    [IntPtr]::Size -ne 8 -or $PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'DESK-15 real recovery requires GitHub-hosted x64 Windows PowerShell STA'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed -PathType Leaf)) { throw 'Not the installed Akula executable' }
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

$root = Join-Path $env:RUNNER_TEMP ('akula-desk15-large-installed-' + [Guid]::NewGuid().ToString('N'))
$vault = Join-Path $root 'vault'
$data = Join-Path $root 'appdata'
[IO.Directory]::CreateDirectory($vault) | Out-Null
[IO.Directory]::CreateDirectory($data) | Out-Null
# Negative control: deliberately make ONLY this disposable data parent
# readable by Everyone. The application must seal its child recovery DACL,
# rather than silently relying on standard LocalAppData inheritance.
$everyone=[System.Security.Principal.SecurityIdentifier]::new('S-1-1-0')
$broadRule=[System.Security.AccessControl.FileSystemAccessRule]::new(
    $everyone,[System.Security.AccessControl.FileSystemRights]::ReadAndExecute,
    [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
    [System.Security.AccessControl.PropagationFlags]::None,
    [System.Security.AccessControl.AccessControlType]::Allow)
$dataAcl=Get-Acl -LiteralPath $data -ErrorAction Stop
$dataAcl.AddAccessRule($broadRule)
Set-Acl -LiteralPath $data -AclObject $dataAcl -ErrorAction Stop
$testRules=(Get-Acl -LiteralPath $data).GetAccessRules(
    $true,$true,[System.Security.Principal.SecurityIdentifier])
$public=@($testRules | Where-Object {
    $_.IdentityReference.Value -eq 'S-1-1-0' -and
    $_.AccessControlType -eq [System.Security.AccessControl.AccessControlType]::Allow
})
if ($public.Count -eq 0) { throw 'Recovery ACL negative control did not create a broad inherited parent' }
Write-Host 'AKULA_WINDOWS_DESK15_ACL_NEGATIVE_CONTROL_PASS disposable_parent_everyone_read=1'
$note = Join-Path $vault 'First note.md'
# Exactly 100000 UTF-8 Unicode lines, >16 MiB and <24 MiB.
# The last line has no newline to make GUI clipboard insertion offsets exact.
$builder = [Text.StringBuilder]::new(21000000)
$filler = 'x' * 185
for ($i=0; $i -lt 100000; $i++) {
    [void]$builder.Append(('Line {0:D6}: ' -f $i))
    [void]$builder.Append($filler)
    [void]$builder.Append(' RU=')
    [void]$builder.Append([char]0x0416)
    [void]$builder.Append([char]0x0438)
    [void]$builder.Append([char]0x0437)
    if ($i -lt 99999) { [void]$builder.Append([char]10) }
}
$initial = $builder.ToString()
$utf8 = [Text.UTF8Encoding]::new($false)
$size = $utf8.GetByteCount($initial)
if ($size -le 16 * 1024 * 1024 -or $size -ge 24 * 1024 * 1024) {
    throw '100000-line Unicode source must be between 16 and 24 MiB'
}
[IO.File]::WriteAllText($note,$initial,[Text.UTF8Encoding]::new($false))
$sourceHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$settings = Join-Path $data 'settings.json'
# A long debounce holds original bytes until a proven snapshot is published.
[IO.File]::WriteAllText($settings,'{"editor":{"saveDebounceMs":120000}}',[Text.UTF8Encoding]::new($false))
$marker = 'DESK15_LARGE_GUI_' + [Guid]::NewGuid().ToString('N')
$shadowBase = Join-Path $data 'akula-note-recovery-v1'
$app = $null

function Start-Installed {
    $launch = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $process = Start-Process -FilePath $installed -ArgumentList $launch -PassThru
    for ($i=0;$i -lt 200;$i++) {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited before installed note window appeared' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and
            $process.MainWindowTitle -like 'First note*' -and
            $process.MainWindowTitle -like '*Akula*') { return $process }
    }
    throw 'Installed Akula did not open the exact recovery fixture'
}
function Assert-Unchanged {
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $sourceHash) {
        throw 'Original file changed before explicit recovery confirmation'
    }
}
function Find-RecoveryButton {
    param([System.Diagnostics.Process]$Process,[string]$Label)
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero -or
        $Process.MainWindowTitle -notlike 'First note*') { throw 'Unexpected UIA target' }
    $rootUi = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Process.MainWindowHandle)
    if ($null -eq $rootUi) { throw 'Installed recovery HWND has no UIA tree' }
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button)
    $buttons = $rootUi.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition)
    $matches = @()
    for ($i=0;$i -lt $buttons.Count;$i++) {
        $item = $buttons.Item($i)
        if ([string]$item.Current.Name -ceq $Label) { $matches+= $item }
    }
    if ($matches.Count -gt 1) { throw 'Ambiguous recovery confirmation buttons' }
    if ($matches.Count -ne 1) { return $null }
    $button = $matches[0]
    $bounds = $button.Current.BoundingRectangle
    if (!$button.Current.IsEnabled -or $button.Current.IsOffscreen -or
        $bounds.IsEmpty -or $bounds.Width -le 0 -or $bounds.Height -le 0) {
        throw 'Recovery UIA action is not visible and enabled'
    }
    return $button
}

function Assert-PrivateShadowAcl {
    param([string]$Base,[string]$Bucket,[string]$Shadow)
    $userSid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $allowed=@($userSid,'S-1-5-18','S-1-5-32-544')
    foreach ($path in @($Base,$Bucket,$Shadow)) {
        $acl=Get-Acl -LiteralPath $path -ErrorAction Stop
        if ($null -eq $acl) { throw 'Recovery ACL descriptor is missing' }
        if ($path -ne $Shadow -and !$acl.AreAccessRulesProtected) {
            throw 'Recovery directory inherits permissions from an untrusted data parent'
        }
        $owner=$acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if ($owner -ne $userSid -and $owner -ne 'S-1-5-32-544') {
            throw 'Recovery file owner is not the current user or local Administrators'
        }
        $rules=$acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])
        $userHasAccess=$false
        foreach ($rule in $rules) {
            if ($rule.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) {
                continue
            }
            $sid=$rule.IdentityReference.Value
            if ($allowed -notcontains $sid) {
                throw 'Recovery ACL exposes plaintext to an unexpected security principal'
            }
            if ($sid -eq $userSid -and
                (($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::ReadData) -ne 0)) {
                $userHasAccess=$true
            }
        }
        if (!$userHasAccess) { throw 'Recovery ACL does not explicitly permit its current user' }
    }
    Write-Host 'AKULA_WINDOWS_DESK15_PRIVATE_ACL_PASS inherited_broad=0 unauthorized_principals=0 paths=3'
}

try {
    $app = Start-Installed
    Send-AkulaBetaText -Process $app -Text $marker
    $shadowOk = $false
    $shadowFile=$null
    for ($i=0;$i -lt 300;$i++) {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Akula crashed before shadow was published' }
        Assert-Unchanged
        if (!(Test-Path -LiteralPath $shadowBase)) { continue }
        $records = @(Get-ChildItem -LiteralPath $shadowBase -Filter '*.shadow' -File -Recurse -ErrorAction Stop)
        foreach ($record in $records) {
            $wire = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($record.FullName))
            if ($wire.Contains($marker) -and $wire.StartsWith('AKULA-NOTE-SHADOW-V1')) {
                $shadowOk = $true
                $shadowFile=$record.FullName
                break
            }
        }
        if ($shadowOk) { break }
    }
    if (!$shadowOk) { throw 'Typed complete draft never reached an fsynced shadow' }
    Assert-PrivateShadowAcl -Base $shadowBase -Bucket (Split-Path -Parent $shadowFile) -Shadow $shadowFile
    Assert-Unchanged
    # Capture the exact dirty editor model before the hard-kill, not an
    # assumed caret position. The model minus its one synthetic marker must
    # reproduce the entire unchanged original source.
    $poisonBefore = 'DESK15_LARGE_PRECRASH_POISON_' + [Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $poisonBefore -ErrorAction Stop
    Send-AkulaBetaShortcut -Process $app -Action SelectAll
    Start-Sleep -Milliseconds 350
    Send-AkulaBetaShortcut -Process $app -Action Copy
    $dirtyModel = $null
    for ($i=0;$i -lt 80;$i++) {
        Start-Sleep -Milliseconds 200
        try {
            $clip = Get-Clipboard -Raw -ErrorAction Stop
            if (![string]::IsNullOrEmpty($clip) -and $clip -cne $poisonBefore -and $clip.Contains($marker)) {
                $dirtyModel = ([regex]::Replace($clip, '\r\n?', [string][char]10)).TrimEnd([char]10)
                break
            }
        } catch [Runtime.InteropServices.ExternalException] { }
    }
    if ($null -eq $dirtyModel) { throw 'Cannot read dirty 100k-line editor model before crash' }
    $at = $dirtyModel.IndexOf($marker,[StringComparison]::Ordinal)
    if ($at -lt 0 -or $at -ne $dirtyModel.LastIndexOf($marker,[StringComparison]::Ordinal) -or
        ![string]::Equals($dirtyModel.Remove($at,$marker.Length),$initial,[StringComparison]::Ordinal)) {
        throw 'Dirty GUI text differs from exactly one marker insertion into 100k source'
    }
    $expected = $initial.Insert($at,$marker)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $expectedHash = ([BitConverter]::ToString($hasher.ComputeHash($utf8.GetBytes($expected)))).Replace('-','')
    } finally { $hasher.Dispose() }
    Assert-Unchanged
    $app.Kill() # Never call graceful CloseMainWindow/Drop/File Gate flush.
    if (!$app.WaitForExit(20000)) { throw 'Hard-killed installed Akula did not exit' }
    $app.Dispose(); $app=$null
    Assert-Unchanged

    # The next session saves approved recovery promptly, unlike the crash run.
    [IO.File]::WriteAllText($settings,'{"editor":{"saveDebounceMs":250}}',[Text.UTF8Encoding]::new($false))
    $app = Start-Installed
    $button = $null
    for ($i=0;$i -lt 100;$i++) {
        Start-Sleep -Milliseconds 200
        Assert-Unchanged
        try { $button=Find-RecoveryButton -Process $app -Label 'Restore draft' }
        catch [System.Windows.Automation.ElementNotAvailableException] { $button=$null }
        if ($null -ne $button) { break }
    }
    if ($null -eq $button) { throw 'Installed app did not offer a real Restore draft UIA button after hard kill' }
    $pattern=$button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $pattern) { throw 'Restore draft lacks the Windows UIA InvokePattern' }
    Assert-Unchanged
    ([System.Windows.Automation.InvokePattern]$pattern).Invoke()

    $saved=$false
    for ($i=0;$i -lt 120;$i++) {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Akula exited during recovery File Gate save' }
        try {
            $disk=[IO.File]::ReadAllText($note)
            if ($disk.Contains($marker) -and $disk.Contains('Line 099999:')) {
                $saved=$true; break
            }
        } catch [IO.IOException] { }
    }
    if (!$saved) { throw 'User-confirmed recovery never reached File Gate disk source' }
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $expectedHash) {
        throw 'Recovered 100k Unicode file has a different full SHA256 than the pre-crash GUI model'
    }
    $clean=$false
    for ($i=0;$i -lt 80;$i++) {
        Start-Sleep -Milliseconds 200
        $left=@(Get-ChildItem -LiteralPath $shadowBase -Filter '*.shadow' -File -Recurse -ErrorAction Stop)
        if ($left.Count -eq 0) { $clean=$true; break }
    }
    if (!$clean) { throw 'Verified File Gate ACK did not retire recovered snapshots' }
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000)) { throw 'Akula refused close after recovery' }
    $app.Dispose(); $app=$null

    $app=Start-Installed
    Start-Sleep -Seconds 2
    if ($null -ne (Find-RecoveryButton -Process $app -Label 'Restore draft')) {
        throw 'Already recovered note incorrectly offered for another recovery'
    }
    $poison='DESK15_CLIPBOARD_POISON_'+[Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $poison -ErrorAction Stop
    Send-AkulaBetaShortcut -Process $app -Action SelectAll
    Start-Sleep -Milliseconds 300
    Send-AkulaBetaShortcut -Process $app -Action Copy
    $readback=$null
    for ($i=0;$i -lt 40;$i++) {
        Start-Sleep -Milliseconds 200
        try {
            $clip=Get-Clipboard -Raw -ErrorAction Stop
            if (![string]::IsNullOrEmpty($clip) -and $clip -cne $poison -and $clip.Contains($marker)) { $readback=$clip; break }
        } catch [System.Runtime.InteropServices.ExternalException] { }
    }
    if ($null -eq $readback) { throw 'Reopened installed GUI failed to copy recovered note' }
    $normalize = {
        param([string]$value)
        ([regex]::Replace($value, '\r\n?', [string][char]10)).TrimEnd([char]10)
    }
    if (![string]::Equals((& $normalize $readback), (& $normalize ([IO.File]::ReadAllText($note))),
        [StringComparison]::Ordinal)) {
        throw 'Reopened editor text does not match exact recovered file content'
    }
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000)) { throw 'Akula refused close after readback' }
    $app.Dispose(); $app=$null
    Write-Host "AKULA_WINDOWS_DESK15_LARGE_RECOVERY_PASS lines=100000 utf8_bytes=$size source_unchanged=1 hard_kill=1 shadow=1 private_acl=1 uia_restore=1 full_sha256_exact=1 gui_exact=1 power_cut=NOT_TESTED"
}
finally {
    if ($null -ne $app) {
        $app.Refresh()
        if (!$app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) }
        $app.Dispose()
    }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
