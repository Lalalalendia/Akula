# Installed Akula P2P identity UI acceptance. CI-only, no synthetic P2P success.
param(
  [Parameter(Mandatory=$true)][string]$ExePath,
  [Parameter(Mandatory=$true)][string]$VaultPath,
  [Parameter(Mandatory=$true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'P2P installed UIA is restricted to disposable GitHub-hosted Windows runners'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
  throw 'P2P UIA requires Windows PowerShell 5.1 STA'
}
$expectedExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
$expectedData = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($expectedExe) -or
    [IO.Path]::GetFullPath($DataDir) -ne [IO.Path]::GetFullPath($expectedData) -or
    !(Test-Path -LiteralPath $expectedExe -PathType Leaf)) {
  throw 'Refusing non-installed or unexpected Akula executable or AppData'
}
$cleanVault = [IO.Path]::GetFullPath($VaultPath).TrimEnd('\')
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
if (!$cleanVault.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase) -or
    !(Test-Path -LiteralPath $cleanVault -PathType Container)) {
  throw 'P2P UIA requires a disposable runner-temp vault'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms -ErrorAction Stop
$locales = @()
foreach ($lang in @('en','ru')) {
  $path = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
  $locales += ([IO.File]::ReadAllText($path, [Text.Encoding]::UTF8) | ConvertFrom-Json)
}
function Names([string]$Key) {
  $found = @()
  foreach ($locale in $locales) {
    $value = [string]$locale.transfer.$Key
    if ([string]::IsNullOrWhiteSpace($value)) { throw "Missing localized transfer key $Key" }
    $found += $value
  }
  return $found
}
$buttonKind = [System.Windows.Automation.PropertyCondition]::new(
  [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
  [System.Windows.Automation.ControlType]::Button)
function Find-Button($Root, [string[]]$Labels) {
  $buttons = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonKind)
  for ($i = $buttons.Count - 1; $i -ge 0; $i--) {
    $candidate = $buttons.Item($i)
    try {
      if ($candidate.Current.IsOffscreen -or !$candidate.Current.IsEnabled) { continue }
      if ($Labels -ccontains [string]$candidate.Current.Name) { return $candidate }
    } catch [System.Windows.Automation.ElementNotAvailableException] {}
  }
  return $null
}
function Wait-Button($Root, [string[]]$Labels) {
  for ($i = 0; $i -lt 100; $i++) {
    $b = Find-Button $Root $Labels
    if ($null -ne $b) { return $b }
    Start-Sleep -Milliseconds 200
  }
  throw ('P2P UIA button not visible: ' + ($Labels -join ' / '))
}
function Invoke-Button($Item) {
  $pattern = $Item.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  if ($null -eq $pattern) { throw "P2P UIA button has no InvokePattern: $($Item.Current.Name)" }
  ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
}
function Assert-No-Unpinned-Transfer($Root) {
  $forbidden = @()
  foreach ($key in @('nativeSendButton','nativeSendInternetButton',
                     'nativeReceiveButton','nativeReceiveInternetButton','inboxSaveCopy')) {
    $forbidden += @(Names $key)
  }
  $all = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonKind)
  for ($i=0; $i -lt $all.Count; $i++) {
    try {
      if ($forbidden -ccontains [string]$all.Item($i).Current.Name) {
        throw 'Unpinned or empty Inbox exposed a P2P transfer/export action'
      }
    } catch [System.Windows.Automation.ElementNotAvailableException] {}
  }
}
function Wait-CopyOrCreate($Root) {
  for ($i=0; $i -lt 100; $i++) {
    if ($null -ne (Find-Button $Root (Names 'localIdentityCopy'))) { return 'ready' }
    if ($null -ne (Find-Button $Root (Names 'localIdentityCreate'))) { return 'missing' }
    Start-Sleep -Milliseconds 200
  }
  throw 'Installed P2P identity UI never reached Ready or Missing state'
}
function Has-UI-Notice($Root, [string[]]$Labels) {
  $all = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    [System.Windows.Automation.Condition]::TrueCondition)
  for ($i=0; $i -lt $all.Count; $i++) {
    try {
      if ($Labels -ccontains [string]$all.Item($i).Current.Name) { return $true }
    } catch [System.Windows.Automation.ElementNotAvailableException] {}
  }
  return $false
}
function Read-PublicIdFromCopy($Root) {
  $poison = 'AKULA_P2P_UIA_CLIPBOARD_BEFORE'
  [System.Windows.Forms.Clipboard]::SetText($poison)
  if ([System.Windows.Forms.Clipboard]::GetText() -cne $poison) {
    throw 'P2P UIA could not establish a known clipboard baseline'
  }
  Invoke-Button (Wait-Button $Root (Names 'localIdentityCopy'))
  $observed = $poison
  $sawUnrelated = $false
  $lastLength = $poison.Length
  for ($i=0; $i -lt 100; $i++) {
    try {
      $observed = [System.Windows.Forms.Clipboard]::GetText()
    } catch [System.Runtime.InteropServices.ExternalException] {
      Start-Sleep -Milliseconds 100
      continue
    }
    if ($observed -cmatch '^[0-9a-f]{64}$' -and $observed -cnotmatch '^0{64}$') {
      return $observed
    }
    if (Has-UI-Notice $Root (Names 'localIdentityCopyFail')) {
      throw 'Installed P2P UI reported an explicit OS clipboard write failure'
    }
    if ($observed -cne $poison) {
      $sawUnrelated = $true
      $lastLength = $observed.Length
    }
    Start-Sleep -Milliseconds 100
  }
  $copyConfirmed = Has-UI-Notice $Root (Names 'localIdentityCopied')
  $copyFailed = Has-UI-Notice $Root (Names 'localIdentityCopyFail')
  throw ("Installed P2P Copy public ID did not yield 64 lowercase hex characters; " +
    "ui_copy_confirmed=$copyConfirmed ui_copy_failed=$copyFailed " +
    "unrelated_clipboard=$sawUnrelated last_length=$lastLength")
}
$note = Join-Path $cleanVault 'p2p-installed-uia-smoke.md'
if (Test-Path -LiteralPath $note) { throw 'P2P UIA note fixture was not clean' }
[IO.File]::WriteAllText($note, '# P2P installed UIA synthetic note', [Text.UTF8Encoding]::new($false))
$noteHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$previousId = $null
$proc = $null
try {
  foreach ($round in 1..2) {
    $args = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $cleanVault,$DataDir,$note
    $proc = Start-Process -FilePath $expectedExe -ArgumentList $args -PassThru
    $ready = $false
    for ($i=0; $i -lt 120; $i++) {
      Start-Sleep -Milliseconds 200
      $proc.Refresh()
      if ($proc.HasExited) { throw "Installed Akula terminated in P2P UIA round $round" }
      if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and $proc.MainWindowTitle -like '*Akula*') {
        $ready = $true
        break
      }
    }
    if (!$ready) { throw 'Installed Akula did not expose its GUI window' }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
    if ($null -eq $root) { throw 'Installed Akula lacks Windows UI Automation root' }
    Invoke-Button (Wait-Button $root (Names 'title'))
    [void](Wait-Button $root (Names 'localIdentityLoad'))
    Assert-No-Unpinned-Transfer $root
    Invoke-Button (Wait-Button $root (Names 'localIdentityLoad'))
    $phase = Wait-CopyOrCreate $root
    if ($phase -eq 'missing') {
      Invoke-Button (Wait-Button $root (Names 'localIdentityCreate'))
    }
    $publicId = Read-PublicIdFromCopy $root
    if ($round -eq 1) {
      $previousId = $publicId
      Write-Host "AKULA_P2P_INSTALLED_UIA_IDENTITY_PASS created=$($phase -eq 'missing') copied=True"
    } elseif ($publicId -cne $previousId) {
      throw 'Installed P2P identity changed across application restart'
    }
    Assert-No-Unpinned-Transfer $root
    Invoke-Button (Wait-Button $root (Names 'close'))
    if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(20000)) {
      throw "Installed Akula did not close in P2P UIA round $round"
    }
    $proc.Dispose()
    $proc = $null
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash) {
      throw 'P2P identity UI modified the synthetic note'
    }
  }
  Write-Host 'AKULA_P2P_INSTALLED_UIA_PASS rounds=2 stable_public_identity=True no_unpinned_actions=True note_unchanged=True'
} finally {
  if ($null -ne $proc) {
    $proc.Refresh()
    if (!$proc.HasExited) {
      $proc.Kill()
      $proc.WaitForExit()
    }
    $proc.Dispose()
  }
  if (Test-Path -LiteralPath $note) { Remove-Item -LiteralPath $note -Force }
}
