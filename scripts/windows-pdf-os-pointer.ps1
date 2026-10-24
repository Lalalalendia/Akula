# Installed Windows OS-pointer acceptance for the FIRST trusted-PDF user flow.
# Uses a disposable GitHub-hosted runner. Never run this on a workstation.
# Real right-click + left-click + keyboard -> installed UI -> File Gate -> search.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir,
    [Parameter(Mandatory = $true)][string]$FixturePdf,
    [Parameter(Mandatory = $true)][string]$ViewerFixturePdf
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'OS pointer PDF acceptance requires a disposable GitHub-hosted Windows runner'
}
if ([IntPtr]::Size -ne 8 -or $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'OS pointer PDF acceptance requires x64 STA Windows PowerShell'
}
$expectedExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expectedExe) -or
    !(Test-Path -LiteralPath $expectedExe -PathType Leaf)) {
    throw 'Refusing an arbitrary Akula executable'
}
if (!(Test-Path -LiteralPath $FixturePdf -PathType Leaf) -or
    !(Test-Path -LiteralPath $ViewerFixturePdf -PathType Leaf) -or
    !(Test-Path -LiteralPath $VaultPath -PathType Container)) {
    throw 'Missing disposable PDF fixture or vault'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

# Mouse INPUT uses a different union layout from KEYBDINPUT (extra info offset 32).
# The two structs have the same Win64 INPUT size (40 bytes).
Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class AkulaPdfPointer {
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(8)] public int MouseX;
        [FieldOffset(12)] public int MouseY;
        [FieldOffset(16)] public uint MouseData;
        [FieldOffset(20)] public uint MouseFlags;
        [FieldOffset(24)] public uint MouseTime;
        [FieldOffset(32)] public IntPtr MouseExtraInfo;
        [FieldOffset(8)] public ushort VirtualKey;
        [FieldOffset(10)] public ushort ScanCode;
        [FieldOffset(12)] public uint KeyFlags;
        [FieldOffset(16)] public uint KeyTime;
        [FieldOffset(24)] public IntPtr KeyExtraInfo;
    }
    [DllImport("user32.dll", SetLastError=true)]
    private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint SendInput(uint count, [In] Input[] input, int size);

    private static void RequireTarget(IntPtr hwnd, int expectedPid) {
        if (hwnd == IntPtr.Zero || Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Unexpected Win64 INPUT layout or window");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("OS pointer target PID mismatch");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("OS pointer target is not the foreground Akula");
    }
    public static void Click(IntPtr hwnd, int expectedPid, int x, int y, bool right) {
        RequireTarget(hwnd, expectedPid);
        if (!SetCursorPos(x, y))
            throw new InvalidOperationException("Cannot place pointer on verified Akula element");
        Thread.Sleep(75);
        RequireTarget(hwnd, expectedPid);
        const uint INPUT_MOUSE = 0;
        uint down = right ? 0x0008u : 0x0002u;
        uint up = right ? 0x0010u : 0x0004u;
        var events = new Input[] {
            new Input { Type=INPUT_MOUSE, MouseFlags=down },
            new Input { Type=INPUT_MOUSE, MouseFlags=up }
        };
        uint sent = SendInput(2, events, Marshal.SizeOf(typeof(Input)));
        if (sent != 2)
            throw new InvalidOperationException("Incomplete real OS mouse click: " + sent);
    }
    public static void QuickOpen(IntPtr hwnd, int expectedPid) {
        RequireTarget(hwnd, expectedPid);
        const uint INPUT_KEYBOARD = 1;
        const uint KEYUP = 2;
        const ushort CTRL = 0x11, O = 0x4f;
        var keys = new Input[] {
            new Input { Type=INPUT_KEYBOARD, VirtualKey=CTRL },
            new Input { Type=INPUT_KEYBOARD, VirtualKey=O },
            new Input { Type=INPUT_KEYBOARD, VirtualKey=O, KeyFlags=KEYUP },
            new Input { Type=INPUT_KEYBOARD, VirtualKey=CTRL, KeyFlags=KEYUP }
        };
        uint sent = SendInput(4, keys, Marshal.SizeOf(typeof(Input)));
        if (sent != 4)
            throw new InvalidOperationException("Incomplete Ctrl+O OS shortcut: " + sent);
    }
}
'@

$sourceName = 'AkulaPDFPointer.PDF'
$source = Join-Path $VaultPath $sourceName
$sentinel = Join-Path $VaultPath 'First note.md'
if (!(Test-Path -LiteralPath $sentinel -PathType Leaf) -or
    (Test-Path -LiteralPath $source)) {
    throw 'Disposable fixture vault is not in the expected initial state'
}
Copy-Item -LiteralPath $FixturePdf -Destination $source -ErrorAction Stop
$sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$nativeViewerName = 'AkulaNativePages.PDF'
$nativeViewerSource = Join-Path $VaultPath $nativeViewerName
if (Test-Path -LiteralPath $nativeViewerSource) { throw 'Duplicate PDF viewer fixture' }
Copy-Item -LiteralPath $ViewerFixturePdf -Destination $nativeViewerSource -ErrorAction Stop
$nativeViewerHash = (Get-FileHash -LiteralPath $nativeViewerSource -Algorithm SHA256).Hash
$previewPattern = 'AkulaPDFPointer PDF *.md'
if (@(Get-ChildItem -LiteralPath $VaultPath -Filter $previewPattern -File).Count -ne 0) {
    throw 'Unexpected old PDF preview in fresh Windows fixture'
}

function Find-Accessible {
    param(
        [Parameter(Mandatory)][System.Windows.Automation.AutomationElement]$Root,
        [Parameter(Mandatory)][string]$Name,
        [System.Windows.Automation.ControlType]$ExpectedType,
        [switch]$Prefix
    )
    $all = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    for ($i = 0; $i -lt $all.Count; $i++) {
        $item = $all.Item($i)
        try {
            $state = $item.Current
            $match = if ($Prefix) { $state.Name.StartsWith($Name, [StringComparison]::Ordinal) }
                     else { [string]::Equals($state.Name, $Name, [StringComparison]::Ordinal) }
            if ($match -and !$state.IsOffscreen -and
                ($null -eq $ExpectedType -or $state.ControlType -eq $ExpectedType)) {
                return $item
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return $null
}

function Wait-Accessible {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [System.Windows.Automation.ControlType]$ExpectedType,
        [switch]$Prefix,
        [int]$Attempts = 100
    )
    for ($i = 0; $i -lt $Attempts; $i++) {
        $found = Find-Accessible -Root $Root -Name $Name -ExpectedType $ExpectedType -Prefix:$Prefix
        if ($null -ne $found) { return $found }
        Start-Sleep -Milliseconds 200
    }
    throw "Installed Akula did not expose expected UI element ($Name)"
}

function Click-Accessible {
    param(
        [System.Windows.Automation.AutomationElement]$Target,
        [System.Windows.Automation.AutomationElement]$Window,
        [System.Diagnostics.Process]$Process,
        [switch]$Right
    )
    $box = $Target.Current.BoundingRectangle
    $windowBox = $Window.Current.BoundingRectangle
    if ($box.IsEmpty -or $box.Width -lt 4 -or $box.Height -lt 4 -or $windowBox.IsEmpty) {
        throw 'UIA target is not a painted rectangle'
    }
    $x = [int][Math]::Round($box.Left + $box.Width / 2)
    $y = [int][Math]::Round($box.Top + $box.Height / 2)
    if ($x -lt $windowBox.Left -or $x -ge $windowBox.Right -or
        $y -lt $windowBox.Top -or $y -ge $windowBox.Bottom) {
        throw 'UIA target click point is outside verified Akula HWND'
    }
    [AkulaPdfPointer]::Click([IntPtr]$Process.MainWindowHandle, $Process.Id, $x, $y, $Right.IsPresent)
}

function Wait-Title {
    param([System.Diagnostics.Process]$Process, [string]$Needle, [int]$Attempts = 90)
    for ($i = 0; $i -lt $Attempts; $i++) {
        Start-Sleep -Milliseconds 200
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Akula exited during installed OS PDF proof' }
        if ($Process.MainWindowTitle.Contains($Needle) -and $Process.MainWindowTitle.Contains('Akula')) {
            return
        }
    }
    throw "Installed Akula did not open expected UI tab ($Needle)"
}

$proc = $null
try {
    $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $sentinel
    $proc = Start-Process -FilePath $expectedExe -ArgumentList $arguments -PassThru
    Wait-Title -Process $proc -Needle 'First note'
    $hwnd = [IntPtr]$proc.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    $foreground = $false
    for ($i = 0; $i -lt 15; $i++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
            $foreground = $true
            break
        }
    }
    if (!$foreground) { throw 'Windows refused focus on verified installed Akula window' }
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $window) { throw 'Akula has no Windows UIA tree' }

    $file = Wait-Accessible -Root $window -Name $sourceName -ExpectedType ([System.Windows.Automation.ControlType]::TreeItem)
    Click-Accessible -Target $file -Window $window -Process $proc -Right
    $menuItem = Wait-Accessible -Root $window -Name ([string]::Concat(@([char]1053,[char]1072,[char]1081,[char]1090,[char]1080,[char]32,[char]1090,[char]1077,[char]1082,[char]1089,[char]1090,[char]32,[char]80,[char]68,[char]70,[char]32,[char]1074,[char]32,[char]65,[char]107,[char]117,[char]108,[char]97))) -ExpectedType ([System.Windows.Automation.ControlType]::MenuItem)
    Click-Accessible -Target $menuItem -Window $window -Process $proc
    [void](Wait-Accessible -Root $window -Name ([string]::Concat(@([char]80,[char]68,[char]70,[char]58,[char]32,[char]1090,[char]1077,[char]1082,[char]1089,[char]1090,[char]1086,[char]1074,[char]1099,[char]1081,[char]32,[char]1087,[char]1086,[char]1080,[char]1089,[char]1082,[char]32,[char]1074,[char]32,[char]65,[char]107,[char]117,[char]108,[char]97))))
    if (@(Get-ChildItem -LiteralPath $VaultPath -Filter $previewPattern -File).Count -ne 0) {
        throw 'PDF was parsed before explicit user confirmation'
    }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $sourceHash) {
        throw 'PDF source changed before confirmation'
    }

    # Modal has focus by product design. Real Windows Enter approves extraction.
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [uint16]0x0d, $false)
    $preview = $null
    for ($i = 0; $i -lt 240; $i++) {
        Start-Sleep -Milliseconds 200
        $matches = @(Get-ChildItem -LiteralPath $VaultPath -Filter $previewPattern -File)
        if ($matches.Count -gt 1) { throw 'One PDF produced duplicate previews' }
        if ($matches.Count -eq 1) {
            $preview = $matches[0]
            break
        }
    }
    if ($null -eq $preview) { throw 'OS-approved PDF extraction never created its preview' }
    $body = [IO.File]::ReadAllText($preview.FullName)
    if (!$body.Contains('AKULANATIVEPDFNEEDLE') -or !$body.Contains('Needs review')) {
        throw 'File Gate preview omitted extracted text or provenance warning'
    }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $sourceHash) {
        throw 'OS PDF import modified the original PDF'
    }
    $previewStem = [IO.Path]::GetFileNameWithoutExtension($preview.Name)
    Wait-Title -Process $proc -Needle $previewStem

    # Switch AWAY from the preview before searching. Otherwise Enter could pass
    # without opening a result, by leaving the imported note already active.
    $firstNote = Wait-Accessible -Root $window -Name 'First note' -ExpectedType ([System.Windows.Automation.ControlType]::TreeItem)
    Click-Accessible -Target $firstNote -Window $window -Process $proc
    Wait-Title -Process $proc -Needle 'First note'
    [AkulaPdfPointer]::QuickOpen($hwnd, $proc.Id)
    # Do not type into a note if Ctrl+O failed to open/focus native search.
    # Locale can vary with Windows UI language; verify either shipped title.
    $searchTitles = @()
    foreach ($lang in @('ru', 'en')) {
        $localePath = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
        $locale = [IO.File]::ReadAllText($localePath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        $searchTitles += [string]$locale.search.title
    }
    $searchReady = $false
    for ($poll = 0; $poll -lt 100; $poll++) {
        $dialogs = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition)
        for ($index = 0; $index -lt $dialogs.Count; $index++) {
            try {
                $element = $dialogs.Item($index).Current
                # Akula declares Role::Dialog; UIA may expose Window, Pane,
                # or Custom depending on the Windows AccessKit adapter.
                if ($searchTitles -contains [string]$element.Name -and !$element.IsOffscreen) {
                    $searchReady = $true
                    break
                }
            } catch [System.Windows.Automation.ElementNotAvailableException] {}
        }
        if ($searchReady) { break }
        Start-Sleep -Milliseconds 200
    }
    if (!$searchReady) { throw 'Ctrl+O did not open the native Search dialog; refusing blind typing' }
    [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $proc.Id, 'AKULANATIVEPDFNEEDLE')

    # A genuine UIA search-result option is required; a file-tree row is not enough.
    [void](Wait-Accessible -Root $window -Name $previewStem -ExpectedType ([System.Windows.Automation.ControlType]::ListItem) -Attempts 260)
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [uint16]0x0d, $false)
    Wait-Title -Process $proc -Needle $previewStem
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $sourceHash -or
        [IO.File]::ReadAllText($preview.FullName) -cne $body) {
        throw 'OS search/open changed the trusted PDF or extracted preview'
    }
    Write-Host 'AKULA_PDF_INSTALLED_POINTER_E2E=PASS right_click=1 menu=1 consent=1 preview=1 search_option=1 enter_open=1 source_unchanged=1'

    # Native viewer proof, separate from Markdown extraction and from --pdf-worker.
    # This follows real OS pointer -> document tab -> UIA visible page -> real key navigation.
    $nativeRow = Wait-Accessible -Root $window -Name $nativeViewerName -ExpectedType ([System.Windows.Automation.ControlType]::TreeItem)
    Click-Accessible -Target $nativeRow -Window $window -Process $proc
    Wait-Title -Process $proc -Needle $nativeViewerName

    function Wait-ViewerPage {
        param([int]$Page, [int]$Attempts = 150)
        for ($poll = 0; $poll -lt $Attempts; $poll++) {
            $proc.Refresh()
            if ($proc.HasExited) { throw 'Akula exited during native PDF page navigation' }
            $pageImage = Find-Accessible -Root $window -Name ('PDF  ' + $nativeViewerName) -ExpectedType ([System.Windows.Automation.ControlType]::Image) -Prefix
            if ($null -ne $pageImage -and $pageImage.Current.Name.Contains(("$Page/2")) -and $pageImage.Current.Name.Contains(([string]::Concat(@([char]1080,[char]1079,[char]1086,[char]1073,[char]1088,[char]1072,[char]1078,[char]1077,[char]1085,[char]1080,[char]1077,[char]32,[char]1075,[char]1086,[char]1090,[char]1086,[char]1074,[char]1086))))) {
                return $pageImage
            }
            Start-Sleep -Milliseconds 200
        }
        throw "Installed native PDF widget failed to show page $Page/2"
    }
    $pdfPage = Wait-ViewerPage -Page 1
    $pdfPage.SetFocus()
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [uint16]0x27, $false)
    $pdfPage = Wait-ViewerPage -Page 2
    $pdfPage.SetFocus()
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [uint16]0x25, $false)
    [void](Wait-ViewerPage -Page 1)
    if ((Get-FileHash -LiteralPath $nativeViewerSource -Algorithm SHA256).Hash -ne $nativeViewerHash) {
        throw 'Native PDF viewing altered its source bytes'
    }
    Write-Host 'AKULA_PDF_INSTALLED_NATIVE_VIEWER=PASS;open=1;pages=2;next=1;previous=1;source_unchanged=1'
} finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (!$proc.HasExited) {
            if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(15000)) {
                $proc.Kill()
                $proc.WaitForExit()
            }
        }
        $proc.Dispose()
    }
}
