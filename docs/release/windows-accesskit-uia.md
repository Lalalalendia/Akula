# ACCESSKIT-WIN-UIA-01: Installed Akula UI Automation acceptance

Status: diagnostic Windows CI gate, not Windows Narrator sign-off. macOS remains frozen.

## Scope

The Rust RenderRoot tests verify the generated AccessKit nodes and internal
Action handling; the pre-existing NSIS Windows beta smoke checks keyboard input
and saving on disk. Neither proves a Windows UIA client can discover and activate
the installed application's Copy buttons.

The script scripts/windows-accesskit-uia.ps1 runs from
scripts/test-windows-beta.ps1 between genuine NSIS installation and the
ordinary edit/restart/uninstall smoke. It runs in a separate STA Windows
PowerShell 5.1 client on a GitHub-hosted ephemeral Windows runner. It refuses
non-runner or unexpected executable paths and never enables installed sync.

Fail-closed requirements:

- Installed Akula process displays the synthetic Markdown fixture.
- A real Windows UIAutomationClient finds exactly two descendant Button
  controls bearing Akula's own RU/EN Copy label, loaded from locale JSON.
- Both are visible, enabled, focusable, have positive bounds intersecting
  the application window, and appear in document order.
- Both expose InvokePattern. A real OS UIA Invoke copies each distinct full
  code body with no fences, verified via the STA Windows clipboard after
  overwriting it with distinct sentinels.
- The synthetic Markdown source hash stays unchanged after both invocations.
- Logs contain only bounded synthetic-fixture diagnostics, no raw UIA dump
  or actual user clipboard contents.

## OS keyboard acceptance (separate from UIA Invoke)

The Windows UIA script also uses AccessKit Focus on the first Copy button
and **real foreground-bound x64 Win32 SendInput** for Enter, Tab, Space and
Shift+Tab. The helper accepts only whitelisted keys, verifies the installed
process HWND/PID and foreground window, then asserts UIA focus traversal,
exact clipboard contents and unchanged Markdown bytes. A separate
`AKULA_WINDOWS_ACCESSKIT_KEYS_PASS` line is mandatory; a successful
InvokePattern test alone no longer suffices.

This still does not prove Narrator speech, a real user's cursor experience
or screen-reader behavior during document replacement and live scrolling.

## Nonclaims and release HOLD

This does not establish audible Narrator navigation/speech, physical Windows
10/11 or scaling behavior, language switching, UIA node lifetime during
physical scrolling, or the suitability of any installer for public release.

Narrator plus Accessibility Insights / Inspect.exe remains a separate
Windows release acceptance criterion in PR #369. The Windows beta is an
unsigned candidate, not a production distribution. MAC-HOLD-01 stays active.

References: Microsoft UI Automation AutomationElement.FromHandle, FindAll,
InvokePattern and AccessKit Windows SubclassingAdapter.
