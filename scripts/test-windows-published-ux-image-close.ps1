# AKULA-INSTALLED-UX-IMAGE-CLOSE-01: verified published NSIS on disposable Windows STA.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:AKULA_DISPOSABLE_UX -ne '1' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
  throw 'AKULA_INSTALLED_UX_HOLD: dedicated GitHub-hosted Windows STA only'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaUxPointer {
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr info);
    public static void Click(int x, int y) {
        if (!SetCursorPos(x, y)) throw new InvalidOperationException("Cannot move to UIA tree item");
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }
}
'@ -ErrorAction Stop

$repo = $env:GITHUB_REPOSITORY
$version = '0.3.12'
$source = 'fa6ec577e55c4ace245c38130cb5ddb07d0c61a3'
$sha = '7bae191d8a9046bee0f10700e3274489ec9da2964d7517fc01dd0adddf335577'
$name = 'Akula_' + $version + '_x64-setup.exe'
$base = Join-Path $env:RUNNER_TEMP 'akula-installed-ux-image-close-0312'
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$data = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
foreach ($p in @($base, $install, $reg, $data)) {
  if (Test-Path -LiteralPath $p) { throw "Not a clean Windows test runner: $p" }
}
New-Item -ItemType Directory -Path $base -Force | Out-Null
$release = gh api "repos/$repo/releases/tags/v$version" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $release.draft -or $release.tag_name -cne "v$version" -or
    $release.target_commitish -cne $source) { throw 'Release source differs from pinned Akula 0.3.12' }
$asset = @($release.assets | Where-Object { $_.name -ceq $name })
if ($asset.Count -ne 1 -or $asset[0].digest -cne "sha256:$sha") {
  throw 'Published Akula installer digest differs'
}
gh release download "v$version" --repo $repo --dir $base --pattern $name
if ($LASTEXITCODE -ne 0) { throw 'Published Akula installer download failed' }
$installer = Join-Path $base $name
if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant() -cne $sha -or
    (Get-Item -LiteralPath $installer).Length -ne $asset[0].size) {
  throw 'Downloaded Windows installer bytes differ'
}
$installed = Start-Process -FilePath $installer -ArgumentList '/S' -PassThru -Wait
if ($installed.ExitCode -ne 0 -or !(Test-Path -LiteralPath $exe -PathType Leaf) -or
    (Get-ItemProperty -Path $reg -Name DisplayVersion).DisplayVersion -cne $version) {
  throw 'Published Akula NSIS installation failed'
}
Write-Host 'AKULA_INSTALLED_UX_PACKAGE_PASS version=0.3.12'

$vault = Join-Path $base 'AkulaUXFixture'
New-Item -ItemType Directory -Path $vault -Force | Out-Null
$note = Join-Path $vault 'README.md'
[System.IO.File]::WriteAllText($note, '# Verify image tab and close last base', [System.Text.UTF8Encoding]::new($false))
$photo = Join-Path $vault 'Photo.png'
$bitmap = [System.Drawing.Bitmap]::new(32, 24)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
  $graphics.Clear([System.Drawing.Color]::CornflowerBlue)
  $bitmap.Save($photo, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
  $graphics.Dispose()
  $bitmap.Dispose()
}
$noteHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$photoHash = (Get-FileHash -LiteralPath $photo -Algorithm SHA256).Hash
$runArgs = '--vault "' + $vault + '" --data-dir "' + $data + '" --open "' + $note + '" --focus'
$proc = Start-Process -FilePath $exe -ArgumentList $runArgs -PassThru
for ($i = 0; $i -lt 100; $i++) {
  Start-Sleep -Milliseconds 250
  $proc.Refresh()
  if ($proc.HasExited) { throw 'Akula exited before UI action' }
  if ($proc.MainWindowHandle -ne [IntPtr]::Zero) { break }
}
if ($proc.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Installed Akula window missing' }
$hwnd = [IntPtr]$proc.MainWindowHandle
$en = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app\native\assets\locales\en.json'
$locale = [System.IO.File]::ReadAllText($en) | ConvertFrom-Json
$closeLabel = [string]$locale.workspaces.closeActive
$openLabel = [string]$locale.common.openFolder
if (!$closeLabel -or !$openLabel) { throw 'Missing English labels for close base and open folder' }
function Find-Node([string]$role, [string]$name, [int]$seconds) {
  $type = [System.Windows.Automation.ControlType]::$role
  $condition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
  for ($attempt = 0; $attempt -lt ($seconds * 4); $attempt++) {
    $proc.Refresh()
    if ($proc.HasExited) { throw "Akula exited awaiting UIA $role $name" }
    try {
      $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
      $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
      for ($n = 0; $n -lt $nodes.Count; $n++) {
        $node = $nodes.Item($n)
        if ($node.Current.Name -ceq $name -and $node.Current.IsEnabled -and !$node.Current.IsOffscreen) {
          return $node
        }
      }
    } catch [System.Windows.Automation.ElementNotAvailableException] {}
    Start-Sleep -Milliseconds 250
  }
  throw "UIA $role '$name' not visible"
}
function Count-Node([string]$role, [string]$name) {
  $type = [System.Windows.Automation.ControlType]::$role
  $cond = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
  $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
  $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
  $count = 0
  for ($n = 0; $n -lt $nodes.Count; $n++) {
    if ($nodes.Item($n).Current.Name -ceq $name) { $count++ }
  }
  return $count
}
function Invoke-Button([string]$name) {
  $button = Find-Node 'Button' $name 30
  $pattern = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  if ($null -eq $pattern) { throw "Installed UIA button '$name' has no InvokePattern" }
  ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
}
function Click-FileTreeItem([string]$name) {
  $item = Find-Node 'TreeItem' $name 35
  $rect = $item.Current.BoundingRectangle
  $window = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd).Current.BoundingRectangle
  $x = [int][Math]::Round($rect.Left + [Math]::Min(40, $rect.Width / 2))
  $y = [int][Math]::Round(($rect.Top + $rect.Bottom) / 2)
  if ($rect.IsEmpty -or $rect.Width -lt 40 -or $rect.Height -lt 8 -or
      $x -le $window.Left -or $x -ge $window.Right -or
      $y -le $window.Top -or $y -ge $window.Bottom) {
    throw "Invalid or unsafe Windows UIA tree item bounds for $name"
  }
  [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
  for ($i = 0; $i -lt 30; $i++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { break }
    Start-Sleep -Milliseconds 200
  }
  if ([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
    throw 'Cannot foreground installed Akula for real pointer click'
  }
  [AkulaUxPointer]::Click($x, $y)
}

$null = Find-Node 'TabItem' 'README' 45
Click-FileTreeItem 'Photo.png'
$null = Find-Node 'TabItem' 'Photo' 40
$null = Find-Node 'TabItem' 'README' 10
$null = Find-Node 'Image' 'Photo.png' 25
if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash -or
    (Get-FileHash -LiteralPath $photo -Algorithm SHA256).Hash -ne $photoHash) {
  throw 'Opening standalone image modified user-owned files'
}
Write-Host 'AKULA_INSTALLED_UX_IMAGE_TAB_PASS image_tab=1 markdown_tab=1 image_accessible=1 source_unchanged=1'

Invoke-Button 'AkulaUXFixture'
Invoke-Button $closeLabel
$null = Find-Node 'Button' $openLabel 40
if ((Count-Node 'TabItem' 'Photo') -ne 0 -or (Count-Node 'TabItem' 'README') -ne 0) {
  throw 'Close last knowledge base left old document tabs'
}
if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash -or
    (Get-FileHash -LiteralPath $photo -Algorithm SHA256).Hash -ne $photoHash) {
  throw 'Close knowledge base altered user files'
}
Write-Host 'AKULA_INSTALLED_UX_CLOSE_LAST_PASS no_vault=1 old_tabs=0 preserved=1'

$proc.Refresh()
if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(25000)) {
  throw 'Akula refused graceful shutdown after close last workspace'
}
if (!(Test-Path -LiteralPath $data -PathType Container)) { throw 'Close base removed user AppData' }
$uninstall = Join-Path $install 'uninstall.exe'
$removed = Start-Process -FilePath $uninstall -ArgumentList '/S' -PassThru -Wait
if ($removed.ExitCode -ne 0 -or (Test-Path -LiteralPath $exe) -or
    !(Test-Path -LiteralPath $data -PathType Container)) {
  throw 'NSIS cleanup did not preserve Akula AppData'
}
if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash -or
    (Get-FileHash -LiteralPath $photo -Algorithm SHA256).Hash -ne $photoHash) {
  throw 'Uninstall changed user-owned workspace'
}
Write-Host 'AKULA_INSTALLED_UX_CLEANUP_PASS uninstall=1 appdata=1 vault=1'
