# QA-only real Windows Notion UI check. No token, network access, or Notion writes.
param([Parameter(Mandatory=$true)][string]$ExePath)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Notion UIA requires an isolated GitHub-hosted Windows runner'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
    throw 'Notion UIA requires Windows PowerShell 5.1 in STA'
}
if (![string]::IsNullOrEmpty($env:AKULA_NOTION_READ_TOKEN)) {
    throw 'A real Notion token is forbidden in this test'
}
$expectedExe=Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\debug\aquilum-native.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expectedExe) -or
    !(Test-Path -LiteralPath $expectedExe)) {
    throw 'Refusing unexpected or missing QA executable'
}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes -ErrorAction Stop
$buttons=[System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button)
function Find-Button {
    param([System.Windows.Automation.AutomationElement]$Tree,
        [string[]]$Names,[bool]$Prefix=$false)
    $nodes=$Tree.FindAll([System.Windows.Automation.TreeScope]::Descendants,$buttons)
    for ($i=$nodes.Count-1;$i -ge 0;$i--) {
        $node=$nodes.Item($i)
        try {
            if ($node.Current.IsOffscreen -or !$node.Current.IsEnabled) { continue }
            $name=[string]$node.Current.Name
            foreach ($wanted in $Names) {
                if (($Prefix -and $name.StartsWith($wanted,[StringComparison]::Ordinal)) -or
                    (!$Prefix -and $name -ceq $wanted)) { return $node }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return $null
}
function Wait-Button {
    param([System.Windows.Automation.AutomationElement]$Tree,
        [string[]]$Names,[bool]$Prefix=$false)
    for ($attempt=0;$attempt -lt 100;$attempt++) {
        $node=Find-Button -Tree $Tree -Names $Names -Prefix $Prefix
        if ($null -ne $node) { return $node }
        Start-Sleep -Milliseconds 200
    }
    throw ('Visible UIA button missing: '+($Names -join ' / '))
}
function Invoke-Button([System.Windows.Automation.AutomationElement]$Node) {
    $pattern=$Node.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    if ($null -eq $pattern) { throw 'UIA InvokePattern unavailable' }
    ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
}
function Visible-Names([System.Windows.Automation.AutomationElement]$Tree) {
    $names=New-Object 'System.Collections.Generic.List[string]'
    $nodes=$Tree.FindAll([System.Windows.Automation.TreeScope]::Descendants,$buttons)
    foreach ($node in $nodes) {
        try {
            if (!$node.Current.IsOffscreen -and $node.Current.IsEnabled) {
                [void]$names.Add([string]$node.Current.Name)
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {}
    }
    return @($names.ToArray())
}
$work=Join-Path $env:RUNNER_TEMP ('akula-notion-uia-'+[guid]::NewGuid().ToString('N'))
$vault=Join-Path $work 'vault'
$data=Join-Path $work 'appdata'
New-Item -ItemType Directory -Force -Path $vault,$data | Out-Null
$note=Join-Path $vault 'Disposable Notion QA.md'
$original='# Offline Notion UIA fixture'+[Environment]::NewLine
[IO.File]::WriteAllText($note,$original,[Text.UTF8Encoding]::new($false))
$locales=@()
foreach ($lang in @('ru','en')) {
    $file=Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $locales+=([IO.File]::ReadAllText($file,[Text.Encoding]::UTF8) | ConvertFrom-Json)
}
$settingsNames=@($locales | ForEach-Object { [string]$_.settings.title })
$syncNames=@($locales | ForEach-Object { [string]$_.settings.nav.sync })
$saveNames=@($locales | ForEach-Object { [string]$_.settings.cloud.save })
$closeNames=@($locales | ForEach-Object { [string]$_.settings.cloud.close })
$proc=$null
try {
    $args='--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,$data,$note
    $proc=Start-Process -FilePath $expectedExe -ArgumentList $args -PassThru
    $window=[IntPtr]::Zero
    for ($attempt=0;$attempt -lt 120;$attempt++) {
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if ($proc.HasExited) { throw ('Akula QA exited: '+$proc.ExitCode) }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like '*Akula*') {
            $window=[IntPtr]$proc.MainWindowHandle
            break
        }
    }
    if ($window -eq [IntPtr]::Zero) { throw 'Native Akula window not found' }
    $root=[System.Windows.Automation.AutomationElement]::FromHandle($window)
    if ($null -eq $root) { throw 'Windows UIA tree unavailable' }
    Invoke-Button (Wait-Button -Tree $root -Names $settingsNames)
    Invoke-Button (Wait-Button -Tree $root -Names $syncNames)
    $tile=Wait-Button -Tree $root -Names @('Notion') -Prefix $true
    if (!$tile.Current.Name.StartsWith('Notion ',[StringComparison]::Ordinal)) {
        throw 'Notion catalog tile lacks provider status'
    }
    Invoke-Button $tile
    $found=$false
    for ($attempt=0;$attempt -lt 100;$attempt++) {
        $names=Visible-Names $root
        if (@($names | Where-Object { $_.Contains('Notion (UUID') }).Count -gt 0) {
            $found=$true
            break
        }
        Start-Sleep -Milliseconds 200
    }
    if (!$found) { throw 'Notion Connect entry point absent from native modal' }
    if ($null -ne (Find-Button -Tree $root -Names $saveNames)) {
        throw 'Notion read-only modal unexpectedly exposes generic Save'
    }
    $names=Visible-Names $root
    if (@($names | Where-Object {
        $_.Contains('Notion API') -or $_.Contains('Akula Bases') -or $_.Contains('JSON')
    }).Count -ne 0) {
        throw 'Unverified source exposes refresh, edit or import actions'
    }
    Write-Host 'AKULA_NOTION_NATIVE_UIA_CATALOG_PASS connect=1'
    Write-Host 'AKULA_NOTION_NATIVE_UIA_NO_SOURCE_PASS import=0'
    Invoke-Button (Wait-Button -Tree $root -Names $closeNames)
    if (Test-Path -LiteralPath (Join-Path $vault 'Notion-import')) {
        throw 'Opening Notion without source created import data'
    }
    if ([IO.File]::ReadAllText($note,[Text.Encoding]::UTF8) -cne $original) {
        throw 'Viewing Notion modified the local fixture'
    }
    Write-Host 'AKULA_NOTION_NATIVE_UIA_NO_WRITE_PASS'
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
