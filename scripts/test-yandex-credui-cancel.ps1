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
using System.Collections.Generic;
using System.Text;
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
    private const int GWL_STYLE = -16;
    private const int ES_PASSWORD = 0x0020;
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "GetClassNameW", SetLastError = true)]
    private static extern int GetClassName(IntPtr hwnd, StringBuilder className, int maxCount);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "GetWindowLongW", SetLastError = true)]
    private static extern int GetWindowStyle(IntPtr hwnd, int offset);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool EnumChildWindows(IntPtr hwndParent, EnumWindowsCallback callback, IntPtr state);

    // Windows UIA does not guarantee every classic native EDIT advertises
    // IsPassword. Check the actual Win32 style instead, never read the
    // credential text. Only windows owned by the exact test PID are visited.
    public static int CountNativeMaskedPasswordEdits(IntPtr root) {
        int found = 0;
        EnumChildWindows(root, delegate (IntPtr hwnd, IntPtr state) {
            var cls = new StringBuilder(128);
            if (GetClassName(hwnd, cls, cls.Capacity) == 0) return true;
            string name = cls.ToString();
            if (string.Equals(name, "Edit", StringComparison.OrdinalIgnoreCase)
                || name.StartsWith("RichEdit", StringComparison.OrdinalIgnoreCase)) {
                if ((GetWindowStyle(hwnd, GWL_STYLE) & ES_PASSWORD) != 0) {
                    found++;
                }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    [DllImport("user32.dll")]
    private static extern int GetDlgCtrlID(IntPtr hwnd);
    // Standard Win32 IDCANCEL=2, independent of the localized button text
    // and of an unhelpful top-level UI Automation provider.
    public static IntPtr[] FindNativeCancelButtons(IntPtr modalRoot) {
        var found = new List<IntPtr>();
        EnumChildWindows(modalRoot, delegate (IntPtr hwnd, IntPtr state) {
            var cls = new StringBuilder(128);
            if (GetClassName(hwnd, cls, cls.Capacity) != 0
                && string.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase)
                && GetDlgCtrlID(hwnd) == 2) {
                found.Add(hwnd);
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static IntPtr[] FindVisibleWindowsForProcess(uint requiredPid) {
        var windows = new List<IntPtr>();
        EnumWindows(delegate (IntPtr hwnd, IntPtr state) {
            uint ownerPid;
            GetWindowThreadProcessId(hwnd, out ownerPid);
            if (ownerPid == requiredPid && IsWindowVisible(hwnd)) {
                windows.Add(hwnd);
            }
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
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
# Cargo's process tree can differ between Windows runner images. Instead of
# trying to locate a cargo-spawned test child, resolve the exact rustc test
# executable through cargo's machine-readable compiler-artifact stream.
$testArgs = @($testFilter, '--exact', '--ignored', '--test-threads=1')
# Cargo returns SUCCESS even when --exact executes zero tests. Verify
# the complete Rust test name before launching the interactive process.
# This script is launched from the repository root by the workflow,
# not from the nested aquilum-app Cargo workspace. Resolve the manifest
# explicitly: discovery failure must never be converted into an empty PASS.
$manifestPath = Join-Path $cargoWorkdir 'Cargo.toml'
$artifactStream = @(& $cargo test --locked --manifest-path $manifestPath -p aquilum-native --bin aquilum-native --features installed,yandex-s3-connection-preview --no-run --message-format=json)
if ($LASTEXITCODE -ne 0) {
    throw 'CredUI Rust test-harness compilation failed'
}
$compiledTests = @($artifactStream | ForEach-Object {
    try {
        $entry = $_ | ConvertFrom-Json -ErrorAction Stop
        if ($entry.reason -eq 'compiler-artifact' -and $entry.target.name -eq 'aquilum-native' -and
            @($entry.target.kind) -contains 'bin' -and $entry.profile.test -eq $true -and
            -not [string]::IsNullOrWhiteSpace([string]$entry.executable)) {
            [string]$entry.executable
        }
    }
    catch [System.ArgumentException] {
        # Cargo progress and diagnostics are not artifact JSON objects.
    }
} | Select-Object -Unique)
if ($compiledTests.Count -ne 1) {
    throw "CredUI exact Rust test artifact not uniquely identified: count=$($compiledTests.Count)"
}
$testExe = [System.IO.Path]::GetFullPath([string]$compiledTests[0])
if (-not (Test-Path -LiteralPath $testExe -PathType Leaf) -or
    [System.IO.Path]::GetExtension($testExe) -ne '.exe' -or
    [System.IO.Path]::GetFileName($testExe) -notmatch '^aquilum[-_]native[-_]') {
    throw 'CredUI resolved executable fails test-harness identity checks'
}
$list = @(& $testExe --list --ignored)
if ($LASTEXITCODE -ne 0) {
    throw 'CredUI Rust test-harness discovery failed'
}
if (-not (@($list) | Where-Object { $_ -eq "$($testFilter): test" })) {
    throw 'CredUI ignored test is missing: refusing false-green zero-test CI'
}
Write-Host 'AKULA_CREDUI_IGNORED_TEST_DISCOVERY_PASS: exact_test_present=True'
$proc = $null
try {
    $proc = Start-Process -FilePath $testExe -ArgumentList $testArgs -WorkingDirectory $cargoWorkdir -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $editType = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $handle = [IntPtr]::Zero
    $dialog = $null
    $masked = 0
    $maskProof = ''
    $maxWindowCount = 0
    $maxEditCount = 0
    for ($attempt = 0; $attempt -lt 240; $attempt++) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) {
            throw "Isolated native CredUI Rust test exited before presenting a masked dialog, code=$($proc.ExitCode)"
        }
        # A Rust GUI test can own multiple visible top-level windows. Do NOT
        # assume EnumWindows' first match is the Windows Security modal.
        # Search only windows of this exact compiled Rust test PID, never
        # other users' credential dialogs or unrelated desktop windows.
        $windows = @([AkulaCredUiQaWindow]::FindVisibleWindowsForProcess([uint32]$proc.Id))
        $maxWindowCount = [Math]::Max($maxWindowCount, $windows.Count)
        foreach ($candidateHandle in $windows) {
            $candidate = [System.Windows.Automation.AutomationElement]::FromHandle($candidateHandle)
            if ($null -eq $candidate) { continue }
            try {
                $candidateFields = $candidate.FindAll(
                    [System.Windows.Automation.TreeScope]::Descendants, $editType
                )
                $maxEditCount = [Math]::Max($maxEditCount, $candidateFields.Count)
                $candidateMasked = 0
                for ($i = 0; $i -lt $candidateFields.Count; $i++) {
                    if ($candidateFields.Item($i).Current.IsPassword) { $candidateMasked++ }
                }
                # The classic Win32 ES_PASSWORD flag is an independent
                # actual-rendering proof if the UIA provider omits IsPassword.
                # Both checks are metadata-only: NEVER read field values.
                $nativeMasked = [AkulaCredUiQaWindow]::CountNativeMaskedPasswordEdits($candidateHandle)
                if ($candidateMasked -ge 1 -or $nativeMasked -ge 1) {
                    $handle = $candidateHandle
                    $dialog = $candidate
                    $masked = [Math]::Max($candidateMasked, $nativeMasked)
                    $maskProof = if ($candidateMasked -ge 1) { 'UIA_IsPassword' } else { 'Win32_ES_PASSWORD' }
                    break
                }
            }
            catch [System.Windows.Automation.ElementNotAvailableException] {
                # A window was destroyed while iterating; retry a fresh tree.
            }
        }
        if ($handle -ne [IntPtr]::Zero) { break }
    }
    if ($handle -eq [IntPtr]::Zero -or $null -eq $dialog -or $masked -lt 1) {
        # Sanitized metadata only; no title, field text, username or secret.
        throw "CredUI masked window unavailable: test_pid_window_count=$maxWindowCount max_edit_controls=$maxEditCount interactive=$([Environment]::UserInteractive)"
    }
    [uint32]$owner = 0
    if ([AkulaCredUiQaWindow]::GetWindowThreadProcessId($handle, [ref]$owner) -eq 0 -or
        $owner -ne [uint32]$proc.Id) {
        throw 'Native credential modal is not owned by the isolated Rust test process'
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
    $activation = $null
    if ($null -ne $cancel) {
        $invoke = $null
        if ($cancel.TryGetCurrentPattern(
            [System.Windows.Automation.InvokePattern]::Pattern, [ref]$invoke
        ) -and $null -ne $invoke) {
            ([System.Windows.Automation.InvokePattern]$invoke).Invoke()
            $activation = 'UIA_Invoke'
        }
    }
    if ($null -eq $activation) {
        # The classic native CredUI IDCANCEL control on this Windows runner
        # does not expose InvokePattern. Microsoft's UIA MSAA proxy can
        # instead expose LegacyIAccessiblePattern.DoDefaultAction(). Only the
        # exact enabled HWND with Win32 IDCANCEL=2 INSIDE the already verified,
        # test-PID-owned masked dialog is eligible. Never send keyboard Escape,
        # BM_CLICK or a synthetic success token.
        foreach ($buttonHwnd in [AkulaCredUiQaWindow]::FindNativeCancelButtons($handle)) {
            $possible = [System.Windows.Automation.AutomationElement]::FromHandle($buttonHwnd)
            if ($null -eq $possible -or -not $possible.Current.IsEnabled) { continue }
            $legacy = $null
            if ($possible.TryGetCurrentPattern(
                [System.Windows.Automation.LegacyIAccessiblePattern]::Pattern,
                [ref]$legacy
            ) -and $null -ne $legacy) {
                ([System.Windows.Automation.LegacyIAccessiblePattern]$legacy).DoDefaultAction()
                $activation = 'UIA_LegacyIAccessible_DefaultAction'
                break
            }
        }
    }
    if ($null -eq $activation) {
        throw 'Genuine Windows CredUI IDCANCEL lacks both UIA Invoke and MSAA DefaultAction'
    }
    if (-not $proc.WaitForExit(30000)) {
        throw 'The genuine Windows CredUI Cancel did not return to the native test'
    }
    if ($proc.ExitCode -ne 0) {
        throw "Windows CredUI Cancel/persistence assertions failed, code=$($proc.ExitCode)"
    }
    # The Rust harness must have actually executed the requested test.
    # Do not upload or print raw GUI logs, even for synthetic credentials.
    $output = [System.IO.File]::ReadAllText($stdout)
    if (-not $output.Contains("test $testFilter ... ok") -or
        $output -notmatch 'test result: ok\. 1 passed; 0 failed') {
        throw 'CredUI Rust harness exited successfully but did not report exactly one passing test'
    }
    # The Rust test confirms Cancel and absence of the disposable WinCred target.
    # Never upload or print raw GUI logs.
    Write-Host "AKULA_YANDEX_CREDUI_CANCEL_PASS: native_modal=True masked_password_fields=$masked mask_proof=$maskProof cancel_activation=$activation no_keys_persisted=True"
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
