# DESK-15: installed real OS typing -> durable shadow -> hard kill -> UIA restore.
# Synthetic hosted Windows CI only; never execute on a developer workstation.
param([Parameter(Mandatory=$true)][string]$ExePath)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    [IntPtr]::Size -ne 8 -or $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'DESK-15 real recovery requires GitHub-hosted x64 Windows PowerShell STA'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed -PathType Leaf)) { throw 'Not the installed Akula executable' }
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

$root = Join-Path $env:RUNNER_TEMP ('akula-desk15-installed-' + [Guid]::NewGuid().ToString('N'))
$vault = Join-Path $root 'vault'
$data = Join-Path $root 'appdata'
[IO.Directory]::CreateDirectory($vault) | Out-Null
[IO.Directory]::CreateDirectory($data) | Out-Null
$note = Join-Path $vault 'First note.md'
$initial = '# First note' + [Environment]::NewLine + [Environment]::NewLine + 'Original source before recovery'
[IO.File]::WriteAllText($note,$initial,[Text.UTF8Encoding]::new($false))
$sourceHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$settings = Join-Path $data 'settings.json'
# A long debounce holds original bytes until a proven snapshot is published.
[IO.File]::WriteAllText($settings,'{"editor":{"saveDebounceMs":120000}}',[Text.UTF8Encoding]::new($false))
$marker = 'DESK15_REAL_GUI_' + [Guid]::NewGuid().ToString('N')
$shadowBase = Join-Path $data 'akula-note-recovery-v1'
$app = $null

function Start-Installed {
    $launch = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $process = Start-Process -FilePath $installed -ArgumentList $launch -PassThru
    for ($i=0;$i -lt 100;$i++) {
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
    $rootUi = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Process.MainWindowHandle)
    if ($null -eq $rootUi) { throw 'Installed recovery HWND has no UIA tree' }
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)
    $buttons = $rootUi.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
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

try {
    $app = Start-Installed
    Send-AkulaBetaText -Process $app -Text $marker
    $shadowOk = $false
    for ($i=0;$i -lt 120;$i++) {
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
                break
            }
        }
        if ($shadowOk) { break }
    }
    if (!$shadowOk) { throw 'Typed complete draft never reached an fsynced shadow' }
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
        catch [Windows.Automation.ElementNotAvailableException] { $button=$null }
        if ($null -ne $button) { break }
    }
    if ($null -eq $button) { throw 'Installed app did not offer a real Restore draft UIA button after hard kill' }
    $pattern=$button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $pattern) { throw 'Restore draft lacks the Windows UIA InvokePattern' }
    Assert-Unchanged
    ([Windows.Automation.InvokePattern]$pattern).Invoke()

    $saved=$false
    for ($i=0;$i -lt 120;$i++) {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Akula exited during recovery File Gate save' }
        try {
            $disk=[IO.File]::ReadAllText($note)
            if ($disk.Contains($marker) -and $disk.Contains('Original source before recovery')) {
                $saved=$true; break
            }
        } catch [IO.IOException] { }
    }
    if (!$saved) { throw 'User-confirmed recovery never reached File Gate disk source' }
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -eq $sourceHash) {
        throw 'Approved recovery did not modify the original'
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
            if ($clip -cne $poison -and $clip.Contains($marker)) { $readback=$clip; break }
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
    Write-Host 'AKULA_WINDOWS_DESK15_INSTALLED_PASS hard_kill=1 shadow=1 uia_confirm=1 file_gate=1 cleanup=1 reopened_gui_exact=1'
}
finally {
    if ($null -ne $app) {
        $app.Refresh()
        if (!$app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) }
        $app.Dispose()
    }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
