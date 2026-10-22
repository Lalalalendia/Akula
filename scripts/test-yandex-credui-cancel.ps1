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
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, Accessibility -ErrorAction Stop
# Windows PowerShell 5.1's managed UIA wrapper lacks LegacyIAccessiblePattern.
# Use the OS MSAA IAccessible COM contract only for the already pinned Cancel HWND.
$accessibilityAssembly = [Accessibility.IAccessible].Assembly.Location
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

    // Hold an independent OS handle from launch until exit status is read.
    // Start-Process -PassThru can discard its managed Process.Handle on PS 5.1.
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(uint desiredAccess, bool inheritHandle, uint processId);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr process);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool IsChild(IntPtr parent, IntPtr possibleChild);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool IsWindowEnabled(IntPtr hwnd);
    [DllImport("oleacc.dll", PreserveSig = true)]
    private static extern int AccessibleObjectFromWindow(
        IntPtr hwnd, uint objectId, ref Guid iid,
        [MarshalAs(UnmanagedType.Interface)] out Accessibility.IAccessible accessible);

    // Native MSAA fallback for an exact child HWND that the caller has
    // already tied to an isolated, masked, same-process CredUI dialog.
    // No BM_CLICK, Escape key, SendMessage or simulated success is permitted.
    public static bool InvokeNativeCancelMsaa(
        IntPtr maskedDialog, IntPtr button, uint requiredPid) {
        if (button == IntPtr.Zero || !IsChild(maskedDialog, button)
            || !IsWindowEnabled(button) || GetDlgCtrlID(button) != 2) return false;
        uint buttonPid;
        if (GetWindowThreadProcessId(button, out buttonPid) == 0
            || buttonPid != requiredPid) return false;
        var cls = new StringBuilder(128);
        if (GetClassName(button, cls, cls.Capacity) == 0
            || !string.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase))
            return false;
        // OBJID_CLIENT = -4 (DWORD 0xFFFFFFFC), IID_IAccessible.
        var iid = new Guid("618736e0-3c3d-11cf-810c-00aa00389b71");
        Accessibility.IAccessible acc = null;
        int hr = AccessibleObjectFromWindow(button, 0xFFFFFFFCu, ref iid, out acc);
        if (hr != 0 || acc == null) return false;
        try {
            acc.accDoDefaultAction(0); // CHILDID_SELF, real OS default action.
            return true;
        } finally {
            if (Marshal.IsComObject(acc)) Marshal.ReleaseComObject(acc);
        }
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
'@ -ReferencedAssemblies $accessibilityAssembly -ErrorAction Stop

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
$processExitHandle = [IntPtr]::Zero
try {
    $proc = Start-Process -FilePath $testExe -ArgumentList $testArgs -WorkingDirectory $cargoWorkdir -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    # PROCESS_QUERY_LIMITED_INFORMATION (0x1000); never open unrelated PIDs.
    # Keep this reference while the OS dialog is active, across process exit.
    $processExitHandle = [AkulaCredUiQaWindow]::OpenProcess(0x1000, $false, [uint32]$proc.Id)
    if ($processExitHandle -eq [IntPtr]::Zero) {
        throw 'Could not retain isolated CredUI test process exit handle'
    }
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
        # The managed UIA wrapper on Windows PowerShell 5.1 does NOT provide
        # LegacyIAccessiblePattern. Ask the native OS MSAA IAccessible provider
        # for the real default action of the same verified IDCANCEL child HWND.
        # The P/Invoke helper rechecks parent, exact PID, class, ID and enabled.
        foreach ($buttonHwnd in [AkulaCredUiQaWindow]::FindNativeCancelButtons($handle)) {
            if ([AkulaCredUiQaWindow]::InvokeNativeCancelMsaa(
                $handle, $buttonHwnd, [uint32]$proc.Id)) {
                $activation = 'MSAA_IAccessible_DefaultAction'
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
    # Query the pinned kernel process handle, not the nullable managed
    # Process.Handle/ExitCode properties after Start-Process -PassThru.
    [uint32]$nativeExitCode = 259
    if (-not [AkulaCredUiQaWindow]::GetExitCodeProcess(
        $processExitHandle, [ref]$nativeExitCode)) {
        throw 'CredUI isolated test OS exit status could not be obtained'
    }
    if ($nativeExitCode -ne 0) {
        # Only fixed-category diagnostics; do not print raw stdout/stderr,
        # credential payloads, dialogs, window titles or panic traces.
        $testOutput = [System.IO.File]::ReadAllText($stdout)
        $exactFailed = $testOutput.Contains("test $testFilter ... FAILED")
        $summaryFailed = $testOutput.Contains('test result: FAILED')
        throw "CredUI genuine test refused: os_exit=$nativeExitCode exact_failed=$exactFailed summary_failed=$summaryFailed"
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
    if ($processExitHandle -ne [IntPtr]::Zero) {
        [AkulaCredUiQaWindow]::CloseHandle($processExitHandle) | Out-Null
    }
    if ($null -ne $proc) {
        $proc.Refresh()
        if (-not $proc.HasExited) {
            $proc.Kill()
            $proc.WaitForExit(5000) | Out-Null
        }
    }
    Remove-Item Env:AKULA_CREDUI_CANCEL_CI -ErrorAction SilentlyContinue
}
