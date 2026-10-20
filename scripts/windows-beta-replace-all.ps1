# Genuine Win32 Ctrl+A/Ctrl+V/Ctrl+Z acceptance on an installed Akula.
# This runs only after NSIS install on a clean GitHub-hosted Windows runner.
# The test creates its *own* disposable vault note; no user notes are used.
function Test-AkulaBetaReplaceAll {
  param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Vault,
    [Parameter(Mandatory)][string]$DataDir
  )
  if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Long-document GUI acceptance is restricted to GitHub-hosted Windows runners'
  }
  if ([IntPtr]::Size -ne 8) { throw 'Expected x64 hosted Windows runner' }
  $note = Join-Path $Vault 'Large paste proof.md'
  if (Test-Path -LiteralPath $note) { throw 'Long-document test fixture already exists' }
  $builder = [System.Text.StringBuilder]::new(950000)
  [void]$builder.Append("# Large paste proof`n")
  for ($i = 0; $i -lt 30000; $i++) {
    [void]$builder.Append("строка $i — проверка окна Markdown редактора`n")
  }
  $original = $builder.ToString()
  if ($original.Length -lt 250000) { throw 'Long-document fixture is too small to move the text window' }
  [System.IO.File]::WriteAllText($note, $original, [System.Text.UTF8Encoding]::new($false))
  $originalHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
  $replacement = 'AKULA_REAL_REPLACE_' + [Guid]::NewGuid().ToString('N')
  $arguments = "--vault `"$Vault`" --data-dir `"$DataDir`" --open `"$note`" --focus"
  $app = $null

  function Await-AkulaLargeNote {
    param([System.Diagnostics.Process]$Process)
    for ($attempt = 0; $attempt -lt 45; $attempt++) {
      Start-Sleep -Milliseconds 500
      $Process.Refresh()
      if ($Process.HasExited) { throw "Installed Akula crashed while opening/handling large note (exit $($Process.ExitCode))" }
      if ($Process.MainWindowHandle -ne [IntPtr]::Zero -and
          $Process.MainWindowTitle -like 'Large paste proof*' -and
          $Process.MainWindowTitle -like '*Akula*') {
        return
      }
    }
    throw 'Installed Akula did not open the expected large Markdown note'
  }

  function Await-AkulaDiskContent {
    param([System.Diagnostics.Process]$Process, [string]$Expected, [string]$Phase)
    # Reading the note is an oracle only: the GUI must make every change.
    for ($attempt = 0; $attempt -lt 45; $attempt++) {
      Start-Sleep -Milliseconds 500
      $Process.Refresh()
      if ($Process.HasExited) { throw "Installed Akula exited during $Phase (exit $($Process.ExitCode))" }
      try {
        $actual = [System.IO.File]::ReadAllText($note)
        if ([string]::Equals($actual.Replace("`r`n", "`n").TrimEnd([char]10),
                             $Expected.Replace("`r`n", "`n").TrimEnd([char]10),
                             [System.StringComparison]::Ordinal)) {
          return
        }
      } catch [System.IO.IOException] {
        # Atomic writer can briefly hold the filesystem source.
      }
    }
    throw "Installed Akula never saved the exact expected text for $Phase"
  }

  try {
    $app = Start-Process -FilePath $Executable -ArgumentList $arguments -PassThru
    Await-AkulaLargeNote -Process $app

    # Ctrl+A moves the global caret to EOF while the loaded editor window
    # starts after byte zero. Ctrl+V used to underflow Document::before(0).
    Set-Clipboard -Value $replacement -ErrorAction Stop
    $clipboard = Get-Clipboard -Raw -ErrorAction Stop
    if ($clipboard.TrimEnd([char[]]@([char]13, [char]10)) -cne $replacement) {
      throw 'Synthetic clipboard contents differ before paste'
    }
    Send-AkulaBetaShortcut -Process $app -Action SelectAll -ExpectedNoteTitle 'Large paste proof'
    Start-Sleep -Milliseconds 300
    Send-AkulaBetaShortcut -Process $app -Action Paste -ExpectedNoteTitle 'Large paste proof'
    Await-AkulaDiskContent -Process $app -Expected $replacement -Phase 'Ctrl+A then Ctrl+V'
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -eq $originalHash) {
      throw 'GUI paste did not change the synthetic long note'
    }

    Send-AkulaBetaShortcut -Process $app -Action Undo -ExpectedNoteTitle 'Large paste proof'
    Await-AkulaDiskContent -Process $app -Expected $original -Phase 'Ctrl+Z restore'
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $originalHash) {
      throw 'Ctrl+Z did not recover exact original source bytes'
    }
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000)) {
      throw 'Installed Akula did not close normally after the replace-all regression'
    }
    $app.Dispose()
    $app = $null

    # The restored document must survive a fresh installed-app launch too.
    $app = Start-Process -FilePath $Executable -ArgumentList $arguments -PassThru
    Await-AkulaLargeNote -Process $app
    $poison = 'AKULA_LONG_READBACK_POISON_' + [Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $poison -ErrorAction Stop
    Send-AkulaBetaShortcut -Process $app -Action SelectAll -ExpectedNoteTitle 'Large paste proof'
    Start-Sleep -Milliseconds 300
    Send-AkulaBetaShortcut -Process $app -Action Copy -ExpectedNoteTitle 'Large paste proof'
    $readbackOk = $false
    for ($poll = 0; $poll -lt 40; $poll++) {
      Start-Sleep -Milliseconds 200
      $app.Refresh()
      if ($app.HasExited) { throw 'Installed Akula exited during large-note GUI readback' }
      try {
        $copied = Get-Clipboard -Raw -ErrorAction Stop
        if ($copied -cne $poison -and
            [string]::Equals($copied.Replace("`r`n", "`n").TrimEnd([char]10),
                             $original.TrimEnd([char]10),
                             [System.StringComparison]::Ordinal)) {
          $readbackOk = $true
          break
        }
      } catch [System.Runtime.InteropServices.ExternalException] {
        # Clipboard may be briefly locked by the app.
      }
    }
    if (!$readbackOk) { throw 'Reopened installed editor did not copy the fully restored note' }
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $originalHash) {
      throw 'Reopen/readback changed the synthetic note'
    }
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000)) {
      throw 'Installed Akula did not close normally after readback'
    }
    $app.Dispose()
    $app = $null
    Write-Host "AKULA_WINDOWS_REPLACE_ALL_PASS bytes=$(([System.Text.Encoding]::UTF8.GetByteCount($original))) native_select=True native_paste=True exact_save=True native_undo=True byte_restore=True reopened_gui_readback=True"
  } finally {
    if ($null -ne $app) {
      $app.Refresh()
      if (!$app.HasExited) {
        $app.Kill()
        $app.WaitForExit()
      }
      $app.Dispose()
    }
  }
}
