# AK-DOC-C5: guided REAL Obsidian Desktop <-> installed Akula Canvas acceptance.
# Run only in an interactive Windows desktop session with both applications.
# This never installs Obsidian, edits production vaults, or runs in required CI.
param(
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [ValidateSet('Prepare','Run')][string]$Mode = 'Run',
    [string]$ObsidianCli,
    [string]$ObsidianVaultName,
    [string]$EvidenceDir
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT' -or $PSVersionTable.PSEdition -ne 'Desktop') {
    throw 'C5 cross-app acceptance requires interactive Windows PowerShell 5.1'
}
$repo = Split-Path -Parent $PSScriptRoot
$pinned = Join-Path $repo 'aquilum-app\core\tests\fixtures\jsoncanvas\official-sample.canvas'
$validator = Join-Path $PSScriptRoot 'canvas-crossapp-validate.cjs'
$vault = [IO.Path]::GetFullPath($VaultPath).TrimEnd('\')
$reference = Join-Path $vault 'reference.canvas'
$marker = Join-Path $vault '.akula-c5-disposable-only'
if (![IO.Path]::GetFileName($vault).StartsWith('akula-c5-', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing non-disposable vault: basename must start with akula-c5-'
}
if (!(Test-Path -LiteralPath $pinned -PathType Leaf)) { throw 'Pinned JSON Canvas fixture absent' }
if (Test-Path -LiteralPath $vault) {
    if ((Get-Item -LiteralPath $vault).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Refusing symlinked/reparse-point vault'
    }
}
if ($Mode -eq 'Prepare') {
    if (Test-Path -LiteralPath $vault) { throw 'Prepare requires a new, nonexistent directory' }
    New-Item -ItemType Directory -Path $vault -Force | Out-Null
    [IO.File]::WriteAllText($marker, 'AKULA-C5-DISPOSABLE-ONLY', [Text.Encoding]::ASCII)
    [IO.File]::WriteAllBytes($reference, [IO.File]::ReadAllBytes($pinned))
    Write-Host ('C5_DISPOSABLE_VAULT_PREPARED '+$vault)
    Write-Host 'Register this exact folder as an Obsidian vault. Enable Obsidian CLI in Settings > General.'
    Write-Host 'Never use a real vault for cross-app acceptance.'
    return
}
if (!(Test-Path -LiteralPath $marker -PathType Leaf) -or
    [IO.File]::ReadAllText($marker) -ne 'AKULA-C5-DISPOSABLE-ONLY') {
    throw 'Disposable vault marker missing or invalid'
}
if (!(Test-Path -LiteralPath $reference -PathType Leaf)) { throw 'Reference canvas missing' }
if ((Get-Item -LiteralPath $reference).Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw 'Reference file must not be a symlink/reparse point'
}
if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($reference)) -ne
    [Convert]::ToBase64String([IO.File]::ReadAllBytes($pinned))) {
    throw 'Reference canvas is not pristine; prepare a NEW vault, never reset the existing one'
}
if ([string]::IsNullOrWhiteSpace($ObsidianCli) -or [string]::IsNullOrWhiteSpace($ObsidianVaultName) -or
    [string]::IsNullOrWhiteSpace($EvidenceDir)) { throw 'Run requires ObsidianCli, ObsidianVaultName and EvidenceDir' }
$cli = [IO.Path]::GetFullPath($ObsidianCli)
if (!(Test-Path -LiteralPath $cli -PathType Leaf) -or
    [IO.Path]::GetFileName($cli) -ine 'Obsidian.com') { throw 'Pass the installed official Obsidian.com CLI redirector' }
if (!(Get-Process -Name Obsidian -ErrorAction SilentlyContinue)) {
    throw 'A running Obsidian Desktop process is required; do not substitute Headless or parsed JSON'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if (!(Test-Path -LiteralPath $installed -PathType Leaf)) { throw 'Installed Akula binary not found' }
if (Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue) {
    throw 'Close all existing Akula instances before the isolated acceptance'
}
if (!(Get-Command node -ErrorAction SilentlyContinue)) { throw 'Node.js is needed for independent acceptance checks' }
$evidence = [IO.Path]::GetFullPath($EvidenceDir).TrimEnd('\')
if (Test-Path -LiteralPath $evidence) { throw 'Evidence output must be a new, nonexistent directory' }
if ($evidence.StartsWith($vault+'\', [StringComparison]::OrdinalIgnoreCase) -or $evidence -ieq $vault) {
    throw 'Evidence must be outside the disposable vault'
}

# Windows CommandLineToArgvW-compatible quoting for Obsidian.com parameters.
# Escape closing quotes and trailing backslashes instead of evaluating shell text.
function Quote-NativeArg([string]$s) {
    $b = New-Object Text.StringBuilder
    [void]$b.Append('"')
    $slashes = 0
    foreach ($ch in $s.ToCharArray()) {
        if ($ch -eq '\') { $slashes++; continue }
        if ($ch -eq '"') {
            [void]$b.Append(('\' * (2*$slashes+1)))
            [void]$b.Append('"')
        } else {
            [void]$b.Append(('\' * $slashes))
            [void]$b.Append($ch)
        }
        $slashes = 0
    }
    [void]$b.Append(('\' * (2*$slashes)))
    [void]$b.Append('"')
    return $b.ToString()
}
function Invoke-Obsidian([string[]]$CliArgs) {
    $temp = Join-Path $env:TEMP ('akula-c5-cli-'+[Guid]::NewGuid().ToString('N'))
    $stdout = $temp+'.out'; $stderr = $temp+'.err'
    $argText = (@("vault=$ObsidianVaultName") + $CliArgs | ForEach-Object { Quote-NativeArg $_ }) -join ' '
    $proc = $null
    try {
        $proc = Start-Process -FilePath $cli -ArgumentList $argText -PassThru -NoNewWindow -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        if (!$proc.WaitForExit(25000)) {
            $proc.Kill(); $proc.WaitForExit()
            throw ('Obsidian CLI timed out: '+($CliArgs -join ' ').Substring(0,[Math]::Min(120,($CliArgs -join ' ').Length)))
        }
        $out = if (Test-Path -LiteralPath $stdout) { [IO.File]::ReadAllText($stdout) } else { '' }
        $err = if (Test-Path -LiteralPath $stderr) { [IO.File]::ReadAllText($stderr) } else { '' }
        if ($proc.ExitCode -ne 0) { throw "Obsidian CLI exit=$($proc.ExitCode): $err" }
        return $out.Trim()
    } finally {
        if ($null -ne $proc) { $proc.Dispose() }
        foreach($file in @($stdout,$stderr)) { if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force } }
    }
}
$reportedVault = (Invoke-Obsidian -CliArgs @('vault','info=path')).Trim().Trim('"').TrimEnd('\')
if ([IO.Path]::GetFullPath($reportedVault).TrimEnd('\') -ine $vault) {
    throw ('Obsidian CLI is connected to a DIFFERENT vault: '+$reportedVault)
}
$obsidianVersion = Invoke-Obsidian -CliArgs @('version')
if ($obsidianVersion -notmatch '\d+\.\d+\.\d+') { throw 'Obsidian version not reported' }
$akulaHash = (Get-FileHash -LiteralPath $installed -Algorithm SHA256).Hash.ToLowerInvariant()
$report = @{
    os = [Environment]::OSVersion.VersionString
    versions = @{ obsidian = $obsidianVersion; akulaBinarySha256 = $akulaHash }
    observations = @{}
    createdUtc = [DateTime]::UtcNow.ToString('o')
}
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
Copy-Item -LiteralPath $reference -Destination (Join-Path $evidence '00-original.canvas')
function Sha([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Record([string]$Phase, [hashtable]$Observation, [string]$Shot) {
    if (!(Test-Path -LiteralPath $Shot -PathType Leaf)) { throw ('GUI screenshot missing: '+$Shot) }
    $bytes = [IO.File]::ReadAllBytes($Shot)
    if ($bytes.Length -le 1024 -or [BitConverter]::ToString($bytes[0..7]) -ne '89-50-4E-47-0D-0A-1A-0A') {
        throw 'GUI screenshot is not a plausible PNG'
    }
    $Observation.screenshot = [IO.Path]::GetFileName($Shot)
    $Observation.screenshotSha256 = Sha $Shot
    $Observation.sha256 = Sha $reference
    $report.observations[$Phase] = $Observation
    [IO.File]::WriteAllText((Join-Path $evidence 'report.json'),
        (ConvertTo-Json $report -Depth 12), [Text.UTF8Encoding]::new($false))
}
# Each Obsidian CLI eval returns a uniquely tagged string. The public CLI
# returns the result of eval, not a guaranteed stream of console.log messages.
# Accept only this invocation's marker, never a stale or unrelated CLI answer.
$obsidianStateCode = '(function(){const l=app.workspace.activeLeaf;const v=l&&l.view;const c=v&&v.canvas;const r=v&&v.containerEl;const state={viewType:v&&v.getViewType(),path:v&&v.file&&v.file.path,nodes:c&&c.nodes&&c.nodes.size,edges:c&&c.edges&&c.edges.size,domNodes:r&&r.querySelectorAll(".canvas-node").length};return "__C5_MARKER__"+JSON.stringify(state);})()'
function Get-ObsidianCanvas([int]$N, [int]$E, [switch]$AlreadyOpen) {
    # Never call 'open' while an operator's Canvas edit could still be saving.
    # For the edited phase, observe the already active native Canvas view.
    if (!$AlreadyOpen) { $null = Invoke-Obsidian -CliArgs @('open','path=reference.canvas') }
    for($i=0;$i -lt 30;$i++) {
        $marker = 'AKULA_C5_STATE_' + [Guid]::NewGuid().ToString('N') + ':'
        $code = $obsidianStateCode.Replace('__C5_MARKER__', $marker)
        $out = Invoke-Obsidian -CliArgs @('eval',"code=$code")
        $raw = $out.Trim()
        # Some CLI output serializers quote a returned string as JSON text.
        # Decode that form once, but never parse unrelated console messages.
        if ($raw.StartsWith('"') -and $raw.EndsWith('"')) {
            $decoded = ConvertFrom-Json -InputObject $raw -ErrorAction Stop
            if ($decoded -isnot [string]) { throw 'Obsidian eval returned a non-string result' }
            $raw = $decoded
        }
        if ($raw.StartsWith($marker, [StringComparison]::Ordinal)) {
            $jsonResult = $raw.Substring($marker.Length)
            $state = ConvertFrom-Json -InputObject $jsonResult -ErrorAction Stop
            if ($state.viewType -eq 'canvas' -and $state.path -eq 'reference.canvas' -and
                $state.nodes -eq $N -and $state.edges -eq $E -and $state.domNodes -eq $N) {
                return $state
            }
        }
        Start-Sleep -Milliseconds 350
    }
    throw "Real Obsidian Canvas DOM/runtime failed to show $N nodes and $E edges"
}
function Capture-Obsidian([string]$Phase, [int]$N, [int]$E, [switch]$AlreadyOpen) {
    $state = Get-ObsidianCanvas $N $E -AlreadyOpen:$AlreadyOpen
    $png = Join-Path $evidence ($Phase+'.png')
    $null = Invoke-Obsidian -CliArgs @('dev:screenshot',"path=$png")
    Record $Phase @{app='obsidian';nodes=$N;edges=$E;viewType=$state.viewType;
        path=$state.path;domNodes=[int]$state.domNodes} $png
    Write-Host ("C5_OBSIDIAN_GUI_OBSERVED phase=$Phase nodes=$N edges=$E")
}
function Close-ObsidianCanvas {
    $js = '(function(){const l=app.workspace.activeLeaf;if(!l||l.view.getViewType()!=="canvas"||l.view.file.path!=="reference.canvas")throw Error("wrong active Canvas");l.detach();return "C5_CLOSED";})()'
    $reply = Invoke-Obsidian -CliArgs @('eval',"code=$js")
    if ($reply -notmatch 'C5_CLOSED') { throw 'Obsidian Canvas was not closed before external editing' }
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaC5Window {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
}
'@
function Start-AkulaCanvas {
    $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault,
        (Join-Path $evidence 'akula-data'),$reference
    return Start-Process -FilePath $installed -ArgumentList $arguments -PassThru
}
function Get-AkulaCanvas([System.Diagnostics.Process]$Process,[int]$N,[int]$E) {
    $label="Canvas: $N nodes, $E edges"
    for($i=0;$i -lt 90;$i++) {
        Start-Sleep -Milliseconds 250
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Installed Akula exited before UIA acceptance' }
        if ($Process.MainWindowHandle -eq [IntPtr]::Zero) { continue }
        $root=[System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Process.MainWindowHandle)
        $cond=[System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty,[string]$label)
        $el=$root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$cond)
        if ($null -eq $el -or $el.Current.IsOffscreen) { continue }
        $bounds=$el.Current.BoundingRectangle; $wnd=$root.Current.BoundingRectangle
        $width=[Math]::Max(0,[Math]::Min($bounds.Right,$wnd.Right)-[Math]::Max($bounds.Left,$wnd.Left))
        $height=[Math]::Max(0,[Math]::Min($bounds.Bottom,$wnd.Bottom)-[Math]::Max($bounds.Top,$wnd.Top))
        if ($width -ge 100 -and $height -ge 100) {
            return @{label=$label; bounds=$bounds; window=$wnd; visible=$true}
        }
    }
    throw "Installed Akula Canvas UIA $label not visible"
}
function Capture-Akula([string]$Phase,[System.Diagnostics.Process]$Process,[int]$N,[int]$E) {
    $ui=Get-AkulaCanvas $Process $N $E
    $hwnd=[IntPtr]$Process.MainWindowHandle
    [AkulaC5Window]::ShowWindow($hwnd,9) | Out-Null
    $focused=$false
    for($i=0;$i -lt 20;$i++) {
        [AkulaC5Window]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 120
        if ([AkulaC5Window]::GetForegroundWindow() -eq $hwnd) { $focused=$true;break }
    }
    if (!$focused) { throw 'Cannot focus installed Akula before screenshot' }
    $rect=$ui.window
    $left=[int][Math]::Max(0,$rect.Left); $top=[int][Math]::Max(0,$rect.Top)
    $w=[int]$rect.Width; $h=[int]$rect.Height
    if ($w -lt 150 -or $h -lt 150 -or $w -gt 8192 -or $h -gt 8192) { throw 'Akula window geometry invalid' }
    $img=New-Object System.Drawing.Bitmap $w,$h
    try {
        $gfx=[Drawing.Graphics]::FromImage($img)
        try { $gfx.CopyFromScreen($left,$top,0,0,$img.Size) }
        finally { $gfx.Dispose() }
        $shot=Join-Path $evidence ($Phase+'.png')
        $img.Save($shot,[Drawing.Imaging.ImageFormat]::Png)
    }finally{ $img.Dispose() }
    Record $Phase @{app='akula';nodes=$N;edges=$E;accessibilityLabel=$ui.label;visibleCanvas=$true} $shot
    Write-Host ("C5_AKULA_INSTALLED_GUI_OBSERVED phase=$Phase nodes=$N edges=$E")
}
function Close-Akula([System.Diagnostics.Process]$Process) {
    if ($null -eq $Process) { return }
    $Process.Refresh()
    if (!$Process.HasExited) {
        if (!$Process.CloseMainWindow() -or !$Process.WaitForExit(10000)) {
            throw 'Akula did not close cleanly. Preserve the isolated vault; do not force-kill it.'
        }
    }
    $Process.Dispose()
}
$akulaProc = $null
try {
    Capture-Obsidian 'obsidian_initial' 5 1
    if ((Sha $reference) -ne (Sha (Join-Path $evidence '00-original.canvas'))) {
        throw 'Obsidian opening official Canvas rewrote source bytes'
    }
    Close-ObsidianCanvas
    $akulaProc = Start-AkulaCanvas
    $null = Get-AkulaCanvas $akulaProc 5 1
    Read-Host 'In installed Akula, click + Text ONCE; wait for save, then press Enter'
    Capture-Akula 'akula_edited' $akulaProc 6 1
    Copy-Item -LiteralPath $reference -Destination (Join-Path $evidence '01-akula.canvas')
    Close-Akula $akulaProc; $akulaProc=$null
    Capture-Obsidian 'obsidian_reopened' 6 1
    if ((Sha $reference) -ne (Sha (Join-Path $evidence '01-akula.canvas'))) {
        throw 'Obsidian reopening Akula edit rewrote source bytes'
    }
    Read-Host 'In Obsidian GUI, add ONE group, ONE text card, ONE new edge to it; wait for save, press Enter'
    Capture-Obsidian 'obsidian_edited' 8 2 -AlreadyOpen
    Copy-Item -LiteralPath $reference -Destination (Join-Path $evidence '02-obsidian.canvas')
    Close-ObsidianCanvas
    $akulaProc = Start-AkulaCanvas
    Capture-Akula 'akula_reopened' $akulaProc 8 2
    if ((Sha $reference) -ne (Sha (Join-Path $evidence '02-obsidian.canvas'))) {
        throw 'Akula reopening Obsidian edit rewrote source bytes'
    }
    Close-Akula $akulaProc; $akulaProc=$null
    & node $validator $evidence $pinned
    if ($LASTEXITCODE -ne 0) { throw 'Strict cross-app evidence validation FAILED' }
    Write-Host ('C5_EVIDENCE_DIR '+$evidence)
} finally {
    if ($null -ne $akulaProc) {
        # Fail closed; do not kill an application containing unsaved user state.
        Write-Warning ('Akula may still be open; inspect it manually. Evidence retained in '+$evidence)
    }
}
