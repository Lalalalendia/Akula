# Windows-only test helper. Inject Unicode keyboard input through the foreground
# OS window, never into a background window or a developer's actual desktop.
# Invoked only by test-windows-beta.ps1 on an ephemeral GitHub-hosted runner.
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'GUI input acceptance is restricted to GitHub-hosted Windows runners'
}
if ([IntPtr]::Size -ne 8) { throw 'GUI input test expects the x64 Windows INPUT layout' }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class AkulaBetaGuiInput {
    // On Windows x64 INPUT has a DWORD type and a 32-byte union at offset 8.
    // Explicit size prevents a truncated KEYBDINPUT-only struct from causing
    // SendInput(ERROR_INVALID_PARAMETER) or writing the wrong window.
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(8)] public ushort VirtualKey;
        [FieldOffset(10)] public ushort ScanCode;
        [FieldOffset(12)] public uint Flags;
        [FieldOffset(16)] public uint Time;
        [FieldOffset(24)] public IntPtr ExtraInfo;
    }

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool ShowWindow(IntPtr hwnd, int command);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint count, [In] Input[] inputs, int size);

    public static void TypeUnicode(IntPtr hwnd, int expectedPid, string text) {
        if (hwnd == IntPtr.Zero || string.IsNullOrEmpty(text))
            throw new InvalidOperationException("Missing window or synthetic text");
        if (Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Unexpected x64 Windows INPUT size");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("Window PID does not match launched Akula");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing to type into a nonforeground window");

        var keys = new Input[text.Length * 2];
        const uint INPUT_KEYBOARD = 1;
        const uint KEYEVENTF_KEYUP = 0x0002;
        const uint KEYEVENTF_UNICODE = 0x0004;
        for (int i = 0; i < text.Length; i++) {
            keys[2 * i] = new Input {
                Type = INPUT_KEYBOARD,
                ScanCode = text[i],
                Flags = KEYEVENTF_UNICODE
            };
            keys[2 * i + 1] = new Input {
                Type = INPUT_KEYBOARD,
                ScanCode = text[i],
                Flags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
            };
        }
        uint sent = SendInput((uint)keys.Length, keys, Marshal.SizeOf(typeof(Input)));
        if (sent != (uint)keys.Length)
            throw new InvalidOperationException("SendInput wrote " + sent + "/" + keys.Length
                + " events; Win32 error " + Marshal.GetLastWin32Error());
    }
}
'@ -ErrorAction Stop

function Send-AkulaBetaText {
  param(
    [Parameter(Mandatory)][System.Diagnostics.Process]$Process,
    [Parameter(Mandatory)][string]$Text
  )
  $Process.Refresh()
  if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
    throw 'Akula is not running with a native top-level window'
  }
  if ($Process.MainWindowTitle -notlike '*Akula*') {
    throw "Unexpected foreground target window title: $($Process.MainWindowTitle)"
  }
  $hwnd = [IntPtr]$Process.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null # SW_RESTORE
  $foreground = $false
  for ($attempt = 0; $attempt -lt 15; $attempt++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 250
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
      $foreground = $true
      break
    }
  }
  if (!$foreground) { throw 'Windows refused to focus the verified Akula window' }
  # --focus targets the actual text editor before this OS-level input.
  [AkulaBetaGuiInput]::TypeUnicode($hwnd, $Process.Id, $Text)
}
