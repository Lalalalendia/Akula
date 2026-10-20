# Actual Windows CredUI Cancel UIA acceptance, only on ephemeral GitHub runner.
# No Yandex credentials, no cloud requests and no non-test vault writes.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'CredUI QA may run only on ephemeral GitHub-hosted Windows runners'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'Native CredUI automation requires STA Windows PowerShell 5.1'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
internal static class AkulaCredUiQaWindow {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindow(string className, string windowTitle);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
}
'@ -ErrorAction Stop

$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoWorkdir = Join-Path $repoRoot 'aquilum-app'
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$title = 'Akula CredUI Cancel QA'
$env:AKULA_CREDUI_CANCEL_CI = '1'
$stdout = Join-Path $env:RUNNER_TEMP 'akula-credui-cancel-qa-stdout.log'
$stderr = Join-Path $env:RUNNER_TEMP 'akula-credui-cancel-qa-stderr.log'
$testFilter = 'yandex_credentials::tests::windows_credui_cancel_real_os_dialog_does_not_persist'
$argv = @('test', '--locked', '-p', 'aquilum-native',
    '--features', 'installed,yandex-s3-connection-preview',
    '--bin', 'aquilum-native', $testFilter,
    '--', '--exact', '--ignored', '--test-threads=1')
$proc = $null
try {
    $proc = Start-Process -FilePath $cargo -ArgumentList $argv -WorkingDirectory $cargoWorkdir -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $handle = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 240; $attempt++) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) {
            throw "Isolated native CredUI test exited before presenting a dialog, code=$($proc.ExitCode)"
        }
        $handle = [AkulaCredUiQaWindow]::FindWindow($null, $title)
        if ($handle -ne [IntPtr]::Zero) { break }
    }
    if ($handle -eq [IntPtr]::Zero) {
        throw 'Akula did not present its native CredUI window with the expected synthetic test caption'
    }
    [uint32]$owner = 0
    if ([AkulaCredUiQaWindow]::GetWindowThreadProcessId($handle, [ref]$owner) -eq 0 -or $owner -eq 0) {
        throw 'Could not identify the native CredUI dialog process'
    }
    # Only act on the actual child launched by the isolated cargo test.
    $testChild = Get-CimInstance Win32_Process -Filter "ProcessId = $owner"
    if ($null -eq $testChild -or [int]$testChild.ParentProcessId -ne $proc.Id) {
        throw 'Native credential dialog is not owned by the isolated cargo test child'
    }
    $dialog = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
    if ($null -eq $dialog -or $dialog.Current.Name -ne $title) {
        throw 'Windows UI Automation cannot identify the actual CredUI dialog'
    }
    $editType = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $fields = $dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editType)
    $masked = 0
    for ($i = 0; $i -lt $fields.Count; $i++) {
        if ($fields.Item($i).Current.IsPassword) { $masked++ }
    }
    if ($masked -lt 1) {
        throw 'Real Microsoft CredUI has no password-masked UI Automation edit field'
    }
    $buttonType = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button
    )
    $buttons = $dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonType)
    $cancel = $null
    for ($i = 0; $i -lt $buttons.Count; $i++) {
        $button = $buttons.Item($i)
        $label = [string]$button.Current.Name
        $automationId = [string]$button.Current.AutomationId
        if ($label -in @('Cancel', 'Отмена') -or $automationId -eq '2') {
            if (-not $button.Current.IsEnabled) {
                throw 'The genuine Windows CredUI Cancel button is disabled'
            }
            $cancel = $button
            break
        }
    }
    if ($null -eq $cancel) { throw 'Genuine Windows CredUI Cancel button not found' }
    $invoke = [System.Windows.Automation.InvokePattern]$cancel.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $invoke) { throw 'Windows CredUI Cancel cannot be activated via UIA' }
    $invoke.Invoke()
    if (-not $proc.WaitForExit(30000)) {
        throw 'The genuine Windows CredUI Cancel did not return to the native test'
    }
    if ($proc.ExitCode -ne 0) {
        throw "Windows CredUI Cancel/persistence assertions failed, code=$($proc.ExitCode)"
    }
    # The Rust test confirms Cancel and absence of the disposable WinCred target.
    # Never upload or print raw GUI logs.
    Write-Host "AKULA_YANDEX_CREDUI_CANCEL_PASS: native_modal=True masked_password_fields=$masked UIA_cancel=True no_keys_persisted=True"
}
finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (-not $proc.HasExited) {
            $proc.Kill()
            $proc.WaitForExit(5000) | Out-Null
        }
    }
    Remove-Item Env:AKULA_CREDUI_CANCEL_CI -ErrorAction SilentlyContinue
}
