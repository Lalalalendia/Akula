# Windows installed-feature Dropbox OAuth browser launch, no Dropbox credentials.
# Only for an ephemeral GitHub-hosted Windows runner. The OAuth browser page
# may open, but this test never consents or reads its address bar/query string.
param([Parameter(Mandatory = $true)][string]$ExePath)
$ErrorActionPreference = 'Stop'

if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Dropbox browser UIA requires an ephemeral hosted Windows runner'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Dropbox browser UIA requires Windows PowerShell 5.1 STA'
}
# CI can invoke cargo from the repository root (workspace/target) or from
# aquilum-app/ (the workspace .cargo/config redirects into .artifacts).
# Resolve the actual target from Cargo and admit only these repository-owned
# build directories. Do not accept an arbitrary path or an external EXE.
$manifest = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app\Cargo.toml'
$metadata = & cargo metadata --locked --no-deps --format-version 1 --manifest-path $manifest |
    ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($metadata.target_directory)) {
    throw 'Cannot resolve the exact candidate target directory via Cargo'
}
$targetDir = [IO.Path]::GetFullPath([string]$metadata.target_directory)
$allowedTargets = @(
    [IO.Path]::GetFullPath((Join-Path $env:GITHUB_WORKSPACE 'aquilum-app\target')),
    [IO.Path]::GetFullPath((Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native'))
)
if ($allowedTargets -notcontains $targetDir) {
    throw 'Refusing a Cargo target directory outside approved Akula build paths'
}
$expected = Join-Path $targetDir 'debug\aquilum-native.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expected) -or
    !(Test-Path -LiteralPath $expected -PathType Leaf)) {
    throw 'Refusing a non-candidate or missing Akula executable'
}

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
$buttonCondition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button)
$windowCondition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Window)
$browserNames = @('msedge','chrome','firefox','brave','vivaldi','opera','iexplore')

function Visible-Button {
    param([System.Windows.Automation.AutomationElement]$Tree,
          [string[]]$Names,
          [bool]$StartsWith = $false)
    $all = $Tree.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        $buttonCondition)
    for ($n = $all.Count - 1; $n -ge 0; $n--) {
        $item = $all.Item($n)
        try {
            if ($item.Current.IsOffscreen -or !$item.Current.IsEnabled) { continue }
            $name = [string]$item.Current.Name
            foreach ($wanted in $Names) {
                if ($StartsWith -and
                    $name.StartsWith($wanted, [StringComparison]::Ordinal)) { return $item }
                if (!$StartsWith -and $name -ceq $wanted) { return $item }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return $null
}
function Wait-Button {
    param([System.Windows.Automation.AutomationElement]$Tree,
          [string[]]$Names,
          [bool]$StartsWith = $false)
    for ($n=0; $n -lt 80; $n++) {
        $button = Visible-Button -Tree $Tree -Names $Names -StartsWith $StartsWith
        if ($null -ne $button) { return $button }
        Start-Sleep -Milliseconds 200
    }
    throw 'Expected Akula settings/Dropbox UIA control is not accessible'
}
function Invoke-Button([System.Windows.Automation.AutomationElement]$Button) {
    $p = [System.Windows.Automation.InvokePattern]$Button.GetCurrentPattern(
        [System.Windows.Automation.InvokePattern]::Pattern)
    $p.Invoke()
}
function Visible-BrowserWindows {
    # Observe only new native top-level windows owned by a known browser
    # process. Do not inspect titles, URLs, process command lines or secrets.
    $found = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal)
    $all = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
        [System.Windows.Automation.TreeScope]::Children, $windowCondition)
    for ($n=0; $n -lt $all.Count; $n++) {
        $item = $all.Item($n)
        try {
            if ($item.Current.IsOffscreen) { continue }
            $rect = $item.Current.BoundingRectangle
            if ($rect.IsEmpty -or $rect.Width -lt 50 -or $rect.Height -lt 50) { continue }
            $ownerId = [int]$item.Current.ProcessId
            $handle = [int]$item.Current.NativeWindowHandle
            if ($ownerId -le 0 -or $handle -eq 0) { continue }
            $owner = Get-Process -Id $ownerId -ErrorAction SilentlyContinue
            if ($null -ne $owner -and $browserNames -ccontains
                $owner.ProcessName.ToLowerInvariant()) {
                [void]$found.Add(('{0}:{1}' -f $ownerId,$handle))
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return ,$found
}

$work = Join-Path $env:RUNNER_TEMP ('akula-dropbox-browser-' +
    [guid]::NewGuid().ToString('N'))
$vault = Join-Path $work 'vault'
$data = Join-Path $work 'appdata'
New-Item -ItemType Directory -Force -Path $vault,$data | Out-Null
$note = Join-Path $vault 'CI Browser Probe.md'
[IO.File]::WriteAllText($note, '# OAuth browser launch fixture' + [Environment]::NewLine,
    [Text.UTF8Encoding]::new($false))
$locales = @()
foreach ($lang in @('ru','en')) {
    $file = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $locales += ([IO.File]::ReadAllText($file, [Text.Encoding]::UTF8) |
        ConvertFrom-Json)
}
$settingsNames = @($locales | ForEach-Object { $_.settings.title })
$syncNames = @($locales | ForEach-Object { $_.settings.nav.sync })
$dropboxNames = @($locales | ForEach-Object { $_.settings.cloud.dropbox })
# PowerShell 5.1 interprets non-BOM script text as ANSI: Unicode escapes only.
$loginName = [regex]::Unescape('\u0412\u043e\u0439\u0442\u0438 \u0432 Dropbox')
$proc = $null
try {
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f
        $vault,$data,$note
    $proc = Start-Process -FilePath $expected -ArgumentList $args -PassThru
    $ready = $false
    for ($n=0; $n -lt 120; $n++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula closed before Dropbox UIA probe' }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Akula main window did not become visible' }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle(
        [IntPtr]$proc.MainWindowHandle)
    if ($null -eq $root) { throw 'Cannot access Akula native UIA tree' }
    Invoke-Button (Wait-Button -Tree $root -Names $settingsNames)
    Invoke-Button (Wait-Button -Tree $root -Names $syncNames)
    Invoke-Button (Wait-Button -Tree $root -Names $dropboxNames -StartsWith $true)
    $login = Wait-Button -Tree $root -Names @($loginName)
    $before = Visible-BrowserWindows
    Invoke-Button $login
    $launched = $false
    for ($n=0; $n -lt 100; $n++) {
        Start-Sleep -Milliseconds 250
        $after = Visible-BrowserWindows
        foreach ($item in $after) {
            if (!$before.Contains($item)) { $launched = $true; break }
        }
        if ($launched) { break }
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula closed after Dropbox login click' }
    }
    if (!$launched) {
        throw 'Dropbox login did not create a visible system browser window'
    }
    Write-Host 'AKULA_DROPBOX_SYSTEM_BROWSER_UIA_PASS browser_window=1 explorer_only=0 consent=0 tokens=0 sync_disabled=1'
}
finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (!$proc.HasExited) {
            if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(10000)) {
                $proc.Kill()
                $proc.WaitForExit()
            }
        }
        $proc.Dispose()
    }
    if (Test-Path -LiteralPath $work) {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}
