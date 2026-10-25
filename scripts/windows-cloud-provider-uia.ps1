# Native Windows cloud-provider UI acceptance without another installed Akula.
# Exercises the actual S3-enabled executable on a disposable, isolated vault.
# UIA Invoke/OS keyboard input are used; no cloud credentials or network writes.
param([Parameter(Mandatory = $true)][string]$ExePath)
$ErrorActionPreference = 'Stop'

if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Cloud UI smoke is restricted to clean GitHub-hosted Windows runners'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Cloud UIA smoke requires Windows PowerShell 5.1 STA'
}
$expectedExe = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\debug\aquilum-native.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expectedExe) -or
    !(Test-Path -LiteralPath $expectedExe)) {
    throw 'Refusing an unexpected or missing S3-featured build'
}

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

$work = Join-Path $env:RUNNER_TEMP ('akula-cloud-uia-' + [guid]::NewGuid().ToString('N'))
$vault = Join-Path $work 'vault'
$data = Join-Path $work 'appdata'
New-Item -ItemType Directory -Force -Path $vault,$data | Out-Null
$note = Join-Path $vault 'Cloud UI test.md'
[IO.File]::WriteAllText($note, '# Disposable cloud UI test' + [Environment]::NewLine,
    [Text.UTF8Encoding]::new($false))

$locales = @()
foreach ($lang in @('ru','en')) {
    $file = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $locales += ([IO.File]::ReadAllText($file, [Text.Encoding]::UTF8) | ConvertFrom-Json)
}
function Labels([scriptblock]$Property) {
    return @($locales | ForEach-Object { & $Property $_ })
}
$settingsNames = @(Labels { param($c) $c.settings.title })
$navNames = @(Labels { param($c) $c.settings.nav.sync })
$saveNames = @(Labels { param($c) $c.settings.cloud.save })
$bucketNames = @(Labels { param($c) $c.settings.yandex.bucket })
$closeNames = @(Labels { param($c) $c.settings.cloud.close })
$yandexNames = @(Labels { param($c) $c.settings.cloud.yandex })
$amazonNames = @(Labels { param($c) $c.settings.cloud.amazon })
$providers = @('yandex','amazon','minio','otherS3','googleDrive','dropbox','webdav')
$buttonsCondition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button)
$editCondition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Edit)

function Find-VisibleButton {
    param([System.Windows.Automation.AutomationElement]$Tree,
        [string[]]$Names, [bool]$StartsWith = $false)
    $all = $Tree.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonsCondition)
    # Last matching active element belongs to the topmost modal overlay.
    for ($n = $all.Count - 1; $n -ge 0; $n--) {
        $item = $all.Item($n)
        try {
            if ($item.Current.IsOffscreen -or !$item.Current.IsEnabled) { continue }
            $name = [string]$item.Current.Name
            foreach ($expected in $Names) {
                if ($StartsWith -and $name.StartsWith($expected, [StringComparison]::Ordinal)) {
                    return $item
                }
                if (!$StartsWith -and $name -ceq $expected) { return $item }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            continue
        }
    }
    return $null
}
function Wait-Button {
    param([System.Windows.Automation.AutomationElement]$Tree,
        [string[]]$Names, [bool]$StartsWith = $false)
    for ($attempt=0; $attempt -lt 60; $attempt++) {
        $item = Find-VisibleButton -Tree $Tree -Names $Names -StartsWith $StartsWith
        if ($null -ne $item) { return $item }
        Start-Sleep -Milliseconds 200
    }
    throw ('Expected accessible button not found: ' + ($Names -join ' / '))
}
function Invoke-Button([System.Windows.Automation.AutomationElement]$Button) {
    $pattern = $Button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $pattern) { throw "No InvokePattern for $($Button.Current.Name)" }
    ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
}
function Wait-ButtonGone {
    param([System.Windows.Automation.AutomationElement]$Tree,[string[]]$Names)
    for ($attempt=0; $attempt -lt 70; $attempt++) {
        if ($null -eq (Find-VisibleButton -Tree $Tree -Names $Names)) { return }
        Start-Sleep -Milliseconds 200
    }
    # Diagnostic output is restricted to disposable CI bucket markers.
    # Never dump arbitrary editor content, imported invitations, or secrets.
    $lastSaved = Saved-Bucket
    $knownNotices = @()
    foreach ($key in @('keysNeedBucket','saveFailed','saveBeforeAction')) {
        foreach ($locale in $locales) {
            try {
                $message = [string]$locale.settings.cloud.$key
                if ([string]::IsNullOrWhiteSpace($message)) { continue }
                if ($null -ne $Tree.FindFirst(
                    [System.Windows.Automation.TreeScope]::Descendants,
                    [System.Windows.Automation.PropertyCondition]::new(
                        [System.Windows.Automation.AutomationElement]::NameProperty, $message))) {
                    $knownNotices += $key
                }
            } catch {}
        }
    }
    throw ('Provider modal did not close: ' + ($Names -join ' / ') +
        '; persistedSyntheticBucket=' + $lastSaved +
        '; knownValidationNotices=' + (($knownNotices | Sort-Object -Unique) -join ','))
}
function Bucket-Editor([System.Windows.Automation.AutomationElement]$Tree) {
    # The underlying note editor can remain visible in UIA behind the
    # focus-trapped cloud modal. Tie the target to the localized bucket row,
    # not to an arbitrary first/last Edit element.
    $all = $Tree.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $labels = @()
    $editors = @()
    foreach ($item in $all) {
        try {
            if ($item.Current.IsOffscreen -or !$item.Current.IsEnabled) { continue }
            $rect = $item.Current.BoundingRectangle
            if ($rect.IsEmpty -or $rect.Height -le 0 -or $rect.Width -le 0) { continue }
            if ($bucketNames -ccontains [string]$item.Current.Name) {
                $labels += $rect
            }
            if ($item.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit -and
                $rect.Height -gt 14 -and $rect.Height -lt 100 -and $rect.Width -gt 60) {
                $editors += $item
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    if ($labels.Count -eq 0) { throw 'Cloud bucket label not accessible in the active modal' }
    $matches = @()
    foreach ($editor in $editors) {
        $r = $editor.Current.BoundingRectangle
        $mid = ($r.Top + $r.Bottom) / 2
        foreach ($label in $labels) {
            $labelMid = ($label.Top + $label.Bottom) / 2
            if ($r.Left -ge $label.Right -and
                ($r.Left - $label.Right) -lt 700 -and
                [Math]::Abs($mid - $labelMid) -le 36) {
                $matches += $editor
                break
            }
        }
    }
    if ($matches.Count -ne 1) {
        throw "Expected one Edit aligned with the localized S3 bucket row; labels=$($labels.Count), edits=$($editors.Count), aligned=$($matches.Count)"
    }
    return $matches[0]
}

function Ensure-Foreground([System.Diagnostics.Process]$Process) {
    $Process.Refresh()
    $hwnd = [IntPtr]$Process.MainWindowHandle
    if ($Process.HasExited -or $hwnd -eq [IntPtr]::Zero) { throw 'Akula window disappeared' }
    [AkulaBetaGuiInput]::ShowWindow($hwnd,9) | Out-Null
    for ($n=0; $n -lt 25; $n++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
        Start-Sleep -Milliseconds 100
    }
    throw 'Cannot focus verified Akula window'
}
function Saved-Bucket {
    $json = Join-Path $data 'settings.json'
    if (!(Test-Path -LiteralPath $json)) { return '' }
    $s = [IO.File]::ReadAllText($json,[Text.Encoding]::UTF8)
    if ($s -match 'cloudtestbucket314') { return 'cloudtestbucket314' }
    if ($s -match 'unsavedbucket315') { return 'unsavedbucket315' }
    return ''
}

$proc = $null
try {
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $proc = Start-Process -FilePath $expectedExe -ArgumentList $args -PassThru
    $ready = $false
    for ($attempt=0; $attempt -lt 120; $attempt++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw "Cloud-enabled Akula exited: $($proc.ExitCode)" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and $proc.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Cloud-enabled Akula did not open its native main window' }
    $hwnd = Ensure-Foreground $proc
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $root) { throw 'Windows UI Automation cannot see Akula' }

    Invoke-Button (Wait-Button -Tree $root -Names $settingsNames)
    Invoke-Button (Wait-Button -Tree $root -Names $navNames)
    foreach ($p in $providers) {
        $names = @(Labels { param($c) $c.settings.cloud.$p })
        # The tile name includes a localized status suffix.
        [void](Wait-Button -Tree $root -Names $names -StartsWith $true)
    }
    Write-Host 'AKULA_CLOUD_CATALOG_UIA_PASS providers=7'

    # Confirm installed Dropbox opens OAuth controls, without invoking a browser.
    $dropboxNames = @(Labels { param($c) $c.settings.cloud.dropbox })
    Invoke-Button (Wait-Button -Tree $root -Names $dropboxNames -StartsWith $true)
    # Windows PowerShell 5.1 reads BOM-less .ps1 as ANSI: use ASCII-only escapes.
    $dropboxSignIn = [regex]::Unescape('\u0412\u043e\u0439\u0442\u0438 \u0432 Dropbox')
    $dropboxRevoke = [regex]::Unescape('\u041e\u0442\u043e\u0437\u0432\u0430\u0442\u044c \u0434\u043e\u0441\u0442\u0443\u043f \u043d\u0430 \u0441\u0430\u0439\u0442\u0435 Dropbox')
    [void](Wait-Button -Tree $root -Names @($dropboxSignIn))
    [void](Wait-Button -Tree $root -Names @($dropboxRevoke))
    [void](Wait-Button -Tree $root -Names $saveNames)
    Invoke-Button (Wait-Button -Tree $root -Names $closeNames)
    Wait-ButtonGone -Tree $root -Names $saveNames
    Write-Host 'AKULA_DROPBOX_MODAL_UIA_PASS oauth_entry=1 no_network=1'

    Invoke-Button (Wait-Button -Tree $root -Names $yandexNames -StartsWith $true)
    [void](Wait-Button -Tree $root -Names $saveNames)
    $editor = Bucket-Editor $root
    $editor.SetFocus()
    $hwnd = Ensure-Foreground $proc
    [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $proc.Id, 'cloudtestbucket314')
    # Check the actual native Accessibility ValuePattern before asking Save
    # to commit. If physical keyboard injection misses the editor, report
    # the typing defect instead of mislabelling it as a persistence failure.
    $observed = ''
    $valuePatternSupported = $false
    try {
        $valuePattern = [System.Windows.Automation.ValuePattern]$editor.GetCurrentPattern(
            [System.Windows.Automation.ValuePattern]::Pattern)
        $valuePatternSupported = $true
        for ($attempt = 0; $attempt -lt 35; $attempt++) {
            $observed = [string]$valuePattern.Current.Value
            if ($observed -ceq 'cloudtestbucket314') { break }
            Start-Sleep -Milliseconds 100
        }
    } catch [System.InvalidOperationException] {
        # Some native Windows accessibility providers do not expose
        # ValuePattern. The authoritative test remains the persisted bytes.
    }
    if ($valuePatternSupported -and $observed -cne 'cloudtestbucket314') {
        throw "Native bucket editor did not receive exact synthetic typing; valueLength=$($observed.Length), expectedLength=18"
    }
    # Typing into the draft is not a settings write.
    if ((Saved-Bucket) -ne '') { throw 'Typing provider draft prematurely saved the bucket' }
    Invoke-Button (Wait-Button -Tree $root -Names $saveNames)
    Wait-ButtonGone -Tree $root -Names $saveNames
    if ((Saved-Bucket) -ne 'cloudtestbucket314') {
        throw 'Save & Close did not persist the cloud bucket to settings.json'
    }
    $tile = Wait-Button -Tree $root -Names $yandexNames -StartsWith $true
    $verifiedLabels = @(Labels { param($c) $c.settings.cloud.verified })
    foreach ($label in $verifiedLabels) {
        if ($tile.Current.Name.Contains($label)) {
            throw 'Saved bucket without remote verification displayed a green connection'
        }
    }
    Write-Host 'AKULA_CLOUD_SAVE_UIA_PASS atomic=True unverified_not_green=True'

    Invoke-Button $tile
    [void](Wait-Button -Tree $root -Names $saveNames)
    $editor = Bucket-Editor $root
    $editor.SetFocus()
    $hwnd = Ensure-Foreground $proc
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [System.UInt16]0x41)
    [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $proc.Id, 'unsavedbucket315')
    Invoke-Button (Wait-Button -Tree $root -Names $closeNames)
    Wait-ButtonGone -Tree $root -Names $saveNames
    if ((Saved-Bucket) -ne 'cloudtestbucket314') {
        throw 'Close without Save changed the persisted bucket'
    }
    Write-Host 'AKULA_CLOUD_DISCARD_UIA_PASS canceled_edit_not_saved=True'

    Invoke-Button (Wait-Button -Tree $root -Names $amazonNames -StartsWith $true)
    [void](Wait-Button -Tree $root -Names $closeNames)
    if ($null -ne (Find-VisibleButton -Tree $root -Names $saveNames)) {
        throw 'Unavailable Amazon provider unexpectedly displayed active Save'
    }
    Invoke-Button (Wait-Button -Tree $root -Names $closeNames)
    Write-Host 'AKULA_CLOUD_UNAVAILABLE_UIA_PASS provider_has_no_write_controls=True'
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
