# Native Windows archive recovery proof; synthetic fixture only, no Notion token.
param([Parameter(Mandatory=$true)][string]$ExePath,
      [Parameter(Mandatory=$true)][string]$FixtureDir)
$ErrorActionPreference='Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Isolated Windows PowerShell STA required'
}
if ($env:AKULA_NOTION_READ_TOKEN) { throw 'Notion token must be absent' }
$expectedExe=Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\debug\aquilum-native.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expectedExe)) {
    throw 'Executable is not the pinned QA build'
}
$rootTemp=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$dir=[IO.Path]::GetFullPath($FixtureDir)
if (!$dir.StartsWith($rootTemp+[IO.Path]::DirectorySeparatorChar,
    [StringComparison]::OrdinalIgnoreCase) -or
    ![IO.Path]::GetFileName($dir).StartsWith('akula-notion-archive-')) {
    throw 'Refusing a fixture outside disposable runner temp'
}
$meta=Get-Content -Raw -LiteralPath (Join-Path $dir 'fixture.json') | ConvertFrom-Json
$vault=Join-Path $dir 'vault'
$data=Join-Path $dir 'appdata'
$note=Join-Path $vault 'Synthetic archive GUI QA.md'
[IO.File]::WriteAllText($note,'# Synthetic recovery test'+[Environment]::NewLine,
    [Text.UTF8Encoding]::new($false))
$original=[IO.File]::ReadAllText($note,[Text.Encoding]::UTF8)
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
$buttons=[System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button)
$scrolls=[System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::IsScrollPatternAvailableProperty,$true)
function Click([System.Windows.Automation.AutomationElement]$Node) {
    $pat=$Node.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    ([System.Windows.Automation.InvokePattern]$pat).Invoke()
}
function Find-Visible([System.Windows.Automation.AutomationElement]$Root,[string]$Contains) {
    $items=$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$buttons)
    for ($i=$items.Count-1;$i -ge 0;$i--) {
        $node=$items.Item($i)
        try {
            if (!$node.Current.IsOffscreen -and $node.Current.IsEnabled -and
                [string]$node.Current.Name -like ('*'+$Contains+'*')) { return $node }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return $null
}
function Next-Scroll([System.Windows.Automation.AutomationElement]$Root) {
    $items=$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$scrolls)
    for ($i=0;$i -lt $items.Count;$i++) {
        try {
            $p=[System.Windows.Automation.ScrollPattern]$items.Item($i).GetCurrentPattern(
                [System.Windows.Automation.ScrollPattern]::Pattern)
            if ($p.Current.VerticallyScrollable -and $p.Current.VerticalScrollPercent -lt 100) {
                $p.ScrollVertical([System.Windows.Automation.ScrollAmount]::LargeIncrement)
                return $true
            }
        } catch [System.Exception] {}
    }
    return $false
}
function Scroll-ToTop([System.Windows.Automation.AutomationElement]$Root) {
    $items=$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$scrolls)
    foreach ($node in $items) {
        try {
            $pat=[System.Windows.Automation.ScrollPattern]$node.GetCurrentPattern(
                [System.Windows.Automation.ScrollPattern]::Pattern)
            if ($pat.Current.VerticallyScrollable -and $pat.Current.VerticalScrollPercent -gt 0) {
                $pat.SetScrollPercent([System.Windows.Automation.ScrollPattern]::NoScroll,0)
            }
        } catch [System.Exception] {}
    }
}
function Visible-Button-Diagnostic([System.Windows.Automation.AutomationElement]$Root) {
    $found=New-Object 'System.Collections.Generic.List[string]'
    $items=$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$buttons)
    for ($i=0;$i -lt $items.Count -and $found.Count -lt 60;$i++) {
        try {
            $node=$items.Item($i)
            $label=[string]$node.Current.Name
            if ($label.Length -gt 100) { $label=$label.Substring(0,100) }
            [void]$found.Add(('{0}: {1}' -f $node.Current.IsOffscreen,$label))
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return ($found -join ' | ')
}
function Wait-Action([System.Windows.Automation.AutomationElement]$Root,[string]$Text) {
    for ($n=0;$n -lt 120;$n++) {
        $node=Find-Visible $Root $Text
        if ($null -ne $node) { return $node }
        if ($n -eq 8) { Scroll-ToTop $Root }
        if (($n % 5) -eq 4 -and $n -ge 9) { [void](Next-Scroll $Root) }
        Start-Sleep -Milliseconds 250
    }
    Write-Host ('NOTION_UIA_VISIBLE_BUTTON_DIAG '+(Visible-Button-Diagnostic $Root))
    $textFilter=[System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Text)
    $texts=$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$textFilter)
    $inspect=New-Object 'System.Collections.Generic.List[string]'
    foreach ($element in $texts) {
        try {
            $name=[string]$element.Current.Name
            if ($name -match 'ревизия|Локальная модель|Проверенный архив|Снимки:|Строк:') {
                if ($name.Length -gt 160) { $name=$name.Substring(0,160) }
                [void]$inspect.Add($name)
                if ($inspect.Count -ge 12) { break }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    Write-Host ('NOTION_UIA_STATE_DIAG '+($inspect -join ' | '))
    throw ('Expected real native button missing: '+$Text)
}
$loc=@()
foreach ($lang in @('ru','en')) {
    $localePath=Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $loc+=([IO.File]::ReadAllText($localePath,[Text.Encoding]::UTF8) | ConvertFrom-Json)
}
$settings=@($loc|ForEach-Object{[string]$_.settings.title})
$sync=@($loc|ForEach-Object{[string]$_.settings.nav.sync})
$process=$null
try {
    $args='--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $process=Start-Process -FilePath $expectedExe -ArgumentList $args -PassThru
    $hwnd=[IntPtr]::Zero
    for ($i=0;$i -lt 120;$i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Native Akula exited unexpectedly' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and
            $process.MainWindowTitle -like '*Akula*') {
            $hwnd=[IntPtr]$process.MainWindowHandle
            break
        }
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw 'Native Akula main window absent' }
    $ui=[System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    $settingsButton=$null
    foreach ($name in $settings) {
        $settingsButton=Find-Visible $ui $name
        if ($null -ne $settingsButton) { break }
    }
    if ($null -eq $settingsButton) { throw 'Settings button absent' }
    Click $settingsButton
    $syncButton=$null
    foreach ($name in $sync) {
        $syncButton=Find-Visible $ui $name
        if ($null -ne $syncButton) { break }
    }
    if ($null -eq $syncButton) { throw 'Sync section absent' }
    Click $syncButton
    Click (Wait-Action $ui 'Notion ')
    Click (Wait-Action $ui $meta.root.Substring(0,8))
    Click (Wait-Action $ui ([regex]::Unescape('\u041e\u0442\u043a\u0440\u044b\u0442\u044c\u0020\u043b\u043e\u043a\u0430\u043b\u044c\u043d\u044b\u0435\u0020\u0041\u006b\u0075\u006c\u0061\u0020\u0042\u0061\u0073\u0065\u0073')))
    Click (Wait-Action $ui $meta.new_digest.Substring(0,10))
    # Read-only fixture guard: distinguish absent local state from a hidden UI control.
    $overlayPath=Join-Path (Join-Path $vault 'Notion-import') ($meta.root+'.akula-bases-edits.json')
    if (!(Test-Path -LiteralPath $overlayPath)) { throw 'Seeded local overlay was lost before GUI recovery' }
    $overlay=Get-Content -Raw -LiteralPath $overlayPath | ConvertFrom-Json
    $sameBaseline=($overlay.baseline_digest -ceq $meta.new_digest)
    Write-Host ('AKULA_NOTION_ARCHIVE_PRECONDITION revision={0} matches_new_digest={1}' -f
        $overlay.revision,$sameBaseline)
    if (!$sameBaseline -or [int64]$overlay.revision -lt 1) {
        throw 'Seeded local overlay has wrong baseline or missing revision'
    }
    Click (Wait-Action $ui ([regex]::Unescape('\u041d\u0430\u0439\u0442\u0438\u0020\u0430\u0440\u0445\u0438\u0432\u044b\u0020\u043b\u043e\u043a\u0430\u043b\u044c\u043d\u044b\u0445\u0020\u043f\u0440\u0430\u0432\u043e\u043a')))
    Click (Wait-Action $ui ([regex]::Unescape('\u0412\u043e\u0441\u0441\u0442\u0430\u043d\u043e\u0432\u0438\u0442\u044c\u0020\u0438\u0437\u0020\u0430\u0440\u0445\u0438\u0432\u0430')))
    $yes=$null
    for ($i=0;$i -lt 80;$i++) {
        $windows=[System.Windows.Automation.AutomationElement]::RootElement.FindAll(
            [System.Windows.Automation.TreeScope]::Children,
            [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($window in $windows) {
            try {
                if ($window.Current.ProcessId -ne $process.Id -or
                    $window.Current.NativeWindowHandle -eq $hwnd.ToInt64()) { continue }
                $choice=Find-Visible $window ([regex]::Unescape('\u0414\u0430'))
                if ($null -eq $choice) { $choice=Find-Visible $window 'Yes' }
                if ($null -ne $choice) { $yes=$choice;break }
            } catch [System.Windows.Automation.ElementNotAvailableException] {}
        }
        if ($null -ne $yes) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $yes) { throw 'OS archive restore Yes/No dialog not found' }
    Click $yes
    Write-Host 'AKULA_NOTION_ARCHIVE_OS_CONFIRMATION_CLICKED'
    # The readback test independently checks exact restored numeric value.
    # This script must never write the archive or overlay directly.
    Start-Sleep -Seconds 2
}
finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (!$process.HasExited) {
            if (!$process.CloseMainWindow() -or !$process.WaitForExit(10000)) {
                $process.Kill();$process.WaitForExit()
            }
        }
        $process.Dispose()
    }
    if ([IO.File]::ReadAllText($note,[Text.Encoding]::UTF8) -cne $original) {
        throw 'Archive workflow unexpectedly modified source note'
    }
}
