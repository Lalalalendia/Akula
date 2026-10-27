# Installed Windows smoke: nine scenario notes and a plain DOCX, all synthetic.
# This is R2 opening/reading evidence, not full mouse-and-keyboard acceptance.
param(
  [Parameter(Mandatory = $true)][string]$ExePath,
  [Parameter(Mandatory = $true)][string]$VaultPath,
  [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA) {
  throw 'Windows scenario readback requires disposable GitHub-hosted Windows STA'
}
if ([IntPtr]::Size -ne 8) { throw 'Expected Windows x64' }
$expectedExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -cne [IO.Path]::GetFullPath($expectedExe) -or
    !(Test-Path -LiteralPath $expectedExe -PathType Leaf)) {
  throw 'Refusing arbitrary executable outside installed Akula'
}
$tempRoot = ([IO.Path]::GetFullPath($env:RUNNER_TEMP)).TrimEnd('\') + '\'
if (!([IO.Path]::GetFullPath($VaultPath)).StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Refusing to use a real user vault'
}
if (!(Test-Path -LiteralPath $VaultPath -PathType Container)) { throw 'Missing isolated vault' }
Add-Type -AssemblyName System.Windows.Forms, UIAutomationClient, UIAutomationTypes -ErrorAction Stop
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
$utf8 = [Text.UTF8Encoding]::new($false)
$examples = [ordered]@{
R01 = @'
# Дневник

Сегодня записал наблюдение.
- [ ] Сверить факты
'@
R02 = @'
# Курс

## Лекция
Определение: альфа и бета.
[[Тема|Конспект]]
'@
R03 = @'
# Исследование

Источник: книга, страница 42.
> Цитата

## Мой вывод
Проверить гипотезу.
'@
R04 = @'
# Справочник

[[Тема#Вывод|Перейти к выводу]]
Ссылки сохраняются.
'@
R05 = @'
# Черновик статьи

## Источники
- [ ] Проверка фактов
- [ ] Вычитка
'@
R06 = @'
# Игровая сессия

[[Персонаж|Арина]] у ворот.

## Итоги
Карту нашли.
'@
R07 = @'
# Инструкция

Команда: systemctl status app

## Решение
Проверить журнал.
'@
R08 = @'
# Проект

[[Решение|Причина переноса]]
- [x] Проверить выпуск
'@
R09 = @'
# Поездка

[[День 1|Первый день]]
Билеты: https://example.org/ticket
Фото хранится локально.
'@
}

function Normalize-CiText {
  param([Parameter(Mandatory = $true)][string]$Value)
  $lf = [string][char]10
  $cr = [string][char]13
  return $Value.Replace($cr + $lf, $lf).Replace($cr, $lf).TrimEnd([char]10)
}

function Open-Installed {
  param([Parameter(Mandatory = $true)][string]$Source)
  $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $Source
  $proc = Start-Process -FilePath $expectedExe -ArgumentList $arguments -PassThru
  $stem = [IO.Path]::GetFileNameWithoutExtension($Source)
  for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    if ($proc.HasExited) { throw 'Installed Akula exited before showing requested document' }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
        $proc.MainWindowTitle.Contains($stem) -and $proc.MainWindowTitle.Contains('Akula')) {
      return $proc
    }
  }
  throw "Installed Akula did not open $stem"
}

function Focus-Installed {
  param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)
  $Process.Refresh()
  if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero -or
      !$Process.MainWindowTitle.Contains('Akula')) { throw 'Installed Akula window unavailable' }
  $hwnd = [IntPtr]$Process.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
  for ($i = 0; $i -lt 20; $i++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 150
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return }
  }
  throw 'Could not focus installed Akula'
}

function Close-Installed {
  param([Diagnostics.Process]$Process)
  if ($null -eq $Process) { return }
  $Process.Refresh()
  if (!$Process.HasExited) {
    if (!$Process.CloseMainWindow() -or !$Process.WaitForExit(15000)) {
      $Process.Kill()
      $Process.WaitForExit()
      throw 'Installed Akula did not close normally'
    }
  }
  $Process.Dispose()
}

$verified = 0
foreach ($route in $examples.Keys) {
  $source = Join-Path $VaultPath ("AKULA_" + $route + "_Проверка.md")
  if (Test-Path -LiteralPath $source) { throw "Nonclean scenario fixture $route" }
  $body = [string]$examples[$route]
  [IO.File]::WriteAllText($source, $body, $utf8)
  $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
  $proc = $null
  try {
    $proc = Open-Installed -Source $source
    Focus-Installed -Process $proc
    $poison = 'AKULA_CLIPBOARD_POISON_' + [Guid]::NewGuid().ToString('N')
    [Windows.Forms.Clipboard]::SetText($poison)
    $hwnd = [IntPtr]$proc.MainWindowHandle
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [uint16]0x41)
    Start-Sleep -Milliseconds 350
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [uint16]0x43)
    $readback = $null
    for ($i = 0; $i -lt 40; $i++) {
      Start-Sleep -Milliseconds 150
      try { $value = [Windows.Forms.Clipboard]::GetText() } catch { $value = '' }
      if (![string]::IsNullOrEmpty($value) -and $value -cne $poison) {
        $readback = $value
        break
      }
    }
    if ($null -eq $readback) { throw "No OS clipboard readback for $route" }
    if ((Normalize-CiText -Value $readback) -cne (Normalize-CiText -Value $body)) {
      throw "Installed GUI returned different text for $route"
    }
  } finally {
    Close-Installed -Process $proc
  }
  if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -cne $hash) {
    throw "Installed GUI changed original $route note"
  }
  $verified++
}
if ($verified -ne 9) { throw "Expected nine Windows readbacks; found $verified" }
Write-Host "AKULA_WINDOWS_SCENARIOS_READBACK_PASS routes=$verified gui_clipboard=1 source_unchanged=1"

# Minimal real Word DOCX package, ZIP/OPC + two unformatted paragraphs.
$docx = Join-Path $VaultPath 'AKULA_DOCX_Простой.docx'
$needle = 'AKULA_DOCX_PLAIN_WIN_841'
$wordXml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>' +
  '<w:p><w:r><w:t>Простой документ из Акулы</w:t></w:r></w:p>' +
  '<w:p><w:r><w:t>' + $needle + '</w:t></w:r></w:p></w:body></w:document>'
$parts = @(
  @{ Name = '[Content_Types].xml'; Text = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>' },
  @{ Name = '_rels/.rels'; Text = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>' },
  @{ Name = 'word/document.xml'; Text = $wordXml }
)
if (Test-Path -LiteralPath $docx) { throw 'Nonclean DOCX fixture' }
$stream = [IO.File]::Open($docx, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite)
try {
  $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $true)
  try {
    foreach ($part in $parts) {
      $entry = $archive.CreateEntry([string]$part.Name, [IO.Compression.CompressionLevel]::Optimal)
      $writer = [IO.StreamWriter]::new($entry.Open(), $utf8)
      try { $writer.Write([string]$part.Text) } finally { $writer.Dispose() }
    }
  } finally { $archive.Dispose() }
} finally { $stream.Dispose() }
$docxHash = (Get-FileHash -LiteralPath $docx -Algorithm SHA256).Hash
$proc = $null
try {
  $proc = Open-Installed -Source $docx
  Focus-Installed -Process $proc
  $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
  if ($null -eq $root) { throw 'Installed DOCX has no UI Automation tree' }
  $textFound = $false
  for ($i = 0; $i -lt 65 -and !$textFound; $i++) {
    Start-Sleep -Milliseconds 200
    $proc.Refresh()
    if ($proc.HasExited) { throw 'Installed Akula crashed while opening DOCX' }
    try {
      $nodes = $root.FindAll([Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition)
      for ($j = 0; $j -lt $nodes.Count; $j++) {
        $element = $nodes.Item($j)
        $name = [string]$element.Current.Name
        if ($name.Contains($needle)) { $textFound = $true; break }
        $value = $element.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty)
        if ($value -is [string] -and $value.Contains($needle)) { $textFound = $true; break }
      }
    } catch [Windows.Automation.ElementNotAvailableException] {
      # The GUI may replace its accessibility tree as the content is rendered.
    }
  }
  if (!$textFound) { throw 'DOCX window appeared but rendered paragraph is not accessible through Windows UIA' }
} finally {
  Close-Installed -Process $proc
}
if ((Get-FileHash -LiteralPath $docx -Algorithm SHA256).Hash -cne $docxHash) {
  throw 'Viewing plain DOCX changed original bytes'
}
Write-Host 'AKULA_WINDOWS_DOCX_PLAIN_READ_PASS installed=1 uia_text=1 source_unchanged=1'
