# Installed Akula Canvas UIA smoke on a disposable GitHub-hosted Windows runner.
# This test uses only synthetic .canvas files and leaves their original bytes unchanged.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Installed Canvas acceptance is restricted to hosted Windows CI'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'Canvas UIA test requires Windows PowerShell STA'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([System.IO.Path]::GetFullPath($ExePath) -ne [System.IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed)) {
    throw 'Refusing to test non-installed Akula'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop

$populatedPath = Join-Path $VaultPath 'canvas-installed-populated.canvas'
$emptyPath = Join-Path $VaultPath 'canvas-installed-empty.canvas'
foreach ($path in @($populatedPath, $emptyPath)) {
    if (Test-Path -LiteralPath $path) { throw "Canvas fixture is not clean: $path" }
}
$populated = @'
{
  "nodes": [
    {"id":"group","type":"group","x":0,"y":0,"width":420,"height":260,"label":"Project"},
    {"id":"first","type":"text","x":24,"y":32,"width":180,"height":100,"text":"First synthetic card"},
    {"id":"second","type":"text","x":230,"y":130,"width":170,"height":90,"text":"Second synthetic card"}
  ],
  "edges": [{"id":"edge","fromNode":"first","toNode":"second","fromSide":"right","toSide":"left","toEnd":"arrow"}],
  "vendor":{"opaque":"preserve"}
}
'@
$utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($populatedPath, $populated, $utf8)
[System.IO.File]::WriteAllText($emptyPath, '{"nodes":[],"edges":[]}', $utf8)
$checks = @(
    @{ Path = $populatedPath; Label = 'Read-only Canvas: 3 nodes, 1 edges'; Opposite = 'Read-only Canvas: 0 nodes, 0 edges' },
    @{ Path = $emptyPath; Label = 'Read-only Canvas: 0 nodes, 0 edges'; Opposite = 'Read-only Canvas: 3 nodes, 1 edges' }
)

foreach ($test in $checks) {
    $originalHash = (Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash
    $proc = $null
    try {
        $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $test.Path
        $proc = Start-Process -FilePath $installed -ArgumentList $arguments -PassThru
        $root = $null
        for ($i = 0; $i -lt 100; $i++) {
            Start-Sleep -Milliseconds 250
            $proc.Refresh()
            if ($proc.HasExited) { throw "Installed Akula exited during Canvas open: $($proc.ExitCode)" }
            if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and $proc.MainWindowTitle -like '*Akula*') {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
                if ($null -ne $root) { break }
            }
        }
        if ($null -eq $root) { throw 'Installed Akula did not expose a Canvas UIA window' }
        $condition = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty,
            [string]$test.Label
        )
        $found = $null
        $visible = $false
        $diagnostic = 'element_not_found'
        for ($i = 0; $i -lt 100; $i++) {
            Start-Sleep -Milliseconds 200
            $proc.Refresh()
            if ($proc.HasExited) { throw 'Akula exited before Canvas accessibility appeared' }
            try {
                $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                if ($null -eq $found) { continue }
                $bounds = $found.Current.BoundingRectangle
                $windowBounds = $root.Current.BoundingRectangle
                # UIA may report a partly clipped child rectangle. Require a
                # substantial, genuinely visible intersection rather than
                # demanding that the complete child fit inside the HWND.
                $width = [Math]::Max(0.0, [Math]::Min($bounds.Right, $windowBounds.Right) -
                    [Math]::Max($bounds.Left, $windowBounds.Left))
                $height = [Math]::Max(0.0, [Math]::Min($bounds.Bottom, $windowBounds.Bottom) -
                    [Math]::Max($bounds.Top, $windowBounds.Top))
                $offscreen = [bool]$found.Current.IsOffscreen
                $diagnostic = ('offscreen={0};canvas=({1},{2},{3},{4});window=({5},{6},{7},{8});intersection=({9},{10})' -f
                    $offscreen, [int]$bounds.Left, [int]$bounds.Top, [int]$bounds.Width, [int]$bounds.Height,
                    [int]$windowBounds.Left, [int]$windowBounds.Top, [int]$windowBounds.Width, [int]$windowBounds.Height,
                    [int]$width, [int]$height)
                if (!$offscreen -and !$bounds.IsEmpty -and !$windowBounds.IsEmpty -and
                    $width -ge 100 -and $height -ge 100) {
                    $visible = $true
                    break
                }
            } catch [System.Windows.Automation.ElementNotAvailableException] {
                $found = $null
                $diagnostic = 'element_temporarily_unavailable'
            }
        }
        if (!$visible) {
            throw "Installed Canvas is not visibly laid out inside its window: $diagnostic"
        }
        $other = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty, [string]$test.Opposite
        )
        if ($null -ne $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $other)) {
            throw 'Canvas accessibility presented stale content from another board'
        }
        if ((Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash -ne $originalHash) {
            throw 'Opening installed Canvas modified the source document'
        }
        Write-Host ("AKULA_WINDOWS_CANVAS_VIEW_PASS file={0} visible=True source_unchanged=True" -f
            [System.IO.Path]::GetFileName($test.Path))
    } finally {
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
    }
    if ((Get-FileHash -LiteralPath $test.Path -Algorithm SHA256).Hash -ne $originalHash) {
        throw 'Closing installed Canvas modified the source document'
    }
}
Write-Host 'AKULA_WINDOWS_CANVAS_INSTALLED_PASS populated=1 empty=1 uia=1 source_sha256=1'
