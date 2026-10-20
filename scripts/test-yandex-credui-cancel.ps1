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
public static class AkulaCredUiQaWindow {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindow(string className, string windowTitle);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    private delegate bool EnumWindowsCallback(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr state);
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);

    // CredUIPromptForCredentialsW may be hosted under a localized or generic
    // Windows Security caption rather than the requested message caption.
    // Enumerate ONLY windows owned by the independently identified cargo-test
    // child; never pick an unrelated process's credential prompt.
    public static IntPtr FindVisibleWindowForProcess(uint requiredPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate (IntPtr hwnd, IntPtr state) {
            uint ownerPid;
            GetWindowThreadProcessId(hwnd, out ownerPid);
            if (ownerPid == requiredPid && IsWindowVisible(hwnd)) {
                found = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
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
# Cargo returns SUCCESS even when --exact executes zero tests. Verify
# the complete Rust test name before launching the interactive process.
$list = & $cargo test --locked -p aquilum-native --bin aquilum-native --features installed,yandex-s3-connection-preview -- --list --ignored
if ($LASTEXITCODE -ne 0) {
    throw 'CredUI test discovery failed'
}
if (-not (@($list) | Where-Object { $_ -eq "$($testFilter): test" })) {
    throw 'CredUI ignored test is missing: refusing false-green zero-test CI'
}
$proc = $null
try {
    $proc = Start-Process -FilePath $cargo -ArgumentList $argv -WorkingDirectory $cargoWorkdir -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $handle = [IntPtr]::Zero
    $observedRustTestProcess = $false
    $discoveredByTestPid = $false
    for ($attempt = 0; $attempt -lt 240; $attempt++) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) {
            throw "Isolated native CredUI test exited before presenting a dialog, code=$($proc.ExitCode)"
        }
        $handle = [AkulaCredUiQaWindow]::FindWindow($null, $title)
        if ($handle -ne [IntPtr]::Zero) { break }
        # A localized Windows Security modal need not expose the requested
        # CredUI caption as its TOP-LEVEL window title. Inspect only visible
        # top-level windows belonging to the cargo-spawned Rust test process.
        if (($attempt % 4) -ne 0) { continue }
        $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($proc.Id)" -ErrorAction SilentlyContinue)
        foreach ($child in $children) {
            if ($child.Name -notmatch '^aquilum[-_]native') { continue }
            $observedRustTestProcess = $true
            $candidate = [AkulaCredUiQaWindow]::FindVisibleWindowForProcess([uint32]$child.ProcessId)
            if ($candidate -ne [IntPtr]::Zero) {
                $handle = $candidate
                $discoveredByTestPid = $true
                break
            }
        }
        if ($handle -ne [IntPtr]::Zero) { break }
    }
    if ($handle -eq [IntPtr]::Zero) {
        throw "Native CredUI window unavailable: interactive=$([Environment]::UserInteractive) rust_test_child_seen=$observedRustTestProcess; runner may not expose interactive desktop"
    }
    [uint32]$owner = 0
    if ([AkulaCredUiQaWindow]::GetWindowThreadProcessId($handle, [ref]$owner) -eq 0 -or $owner -eq 0) {
        throw 'Could not identify the native CredUI dialog process'
    }
    # Only act on the actual child launched by the isolated cargo test.
    $testChild = Get-CimInstance Win32_Process -Filter "ProcessId = $owner"
    if ($null -eq $testChild -or [int]$testChild.ParentProcessId -ne $proc.Id -or
        $testChild.Name -notmatch '^aquilum[-_]native') {
        throw 'Native credential dialog is not owned by the isolated Rust test child'
    }
    $dialog = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
    if ($null -eq $dialog) {
        throw 'Windows UI Automation cannot access the test-child modal'
    }
    if ($dialog.Current.Name -ne $title -and -not $discoveredByTestPid) {
        throw 'CredUI dialog caption mismatch outside the test-child fallback'
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
        # PowerShell 5.1 reads UTF-8 source without BOM using the local ANSI code page.
        # Use ASCII-only literals; AutomationId 2 identifies native IDCANCEL.
        if ($label -eq 'Cancel' -or $automationId -eq '2') {
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
