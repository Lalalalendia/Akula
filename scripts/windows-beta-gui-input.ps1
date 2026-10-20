# Windows-only test helper. Inject actual physical-keyboard-style events
# into the verified foreground Akula window. No direct note writes.
# Invoked ONLY on the ephemeral GitHub-hosted Windows x64 runner.
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'GUI input acceptance is restricted to GitHub-hosted Windows runners'
}
if ([IntPtr]::Size -ne 8) { throw 'GUI input test expects the x64 Windows INPUT layout' }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class AkulaBetaGuiInput {
    // Windows x64 INPUT has a DWORD type and 32-byte union at offset 8.
    // Do not pass a truncated KEYBDINPUT struct to SendInput.
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

    [DllImport("user32.dll")]
    static extern IntPtr GetKeyboardLayout(uint threadId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    static extern short VkKeyScanExW(char ch, IntPtr keyboardLayout);

    [DllImport("user32.dll", ExactSpelling = true)]
    static extern uint MapVirtualKeyExW(uint code, uint mapType, IntPtr keyboardLayout);

    [DllImport("user32.dll")]
    static extern short GetKeyState(int virtualKey);

    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint count, [In] Input[] inputs, int size);

    // Injected Unicode packets generate synthetic unidentified keys
    // with no IME commit on winit 0.31. Use ordinary virtual keys and the
    // foreground application's active keyboard layout instead.
    // Only the three navigation/activation keys required for the isolated
    // AccessKit smoke. Never expose arbitrary system-wide key injection.
    public static void SendNavigationKey(IntPtr hwnd, int expectedPid, ushort key, bool shift) {
        const ushort VK_TAB = 0x09;
        const ushort VK_RETURN = 0x0d;
        const ushort VK_SPACE = 0x20;
        const ushort VK_SHIFT = 0x10;
        const uint INPUT_KEYBOARD = 1;
        const uint KEYEVENTF_KEYUP = 0x0002;
        if (key != VK_TAB && key != VK_RETURN && key != VK_SPACE)
            throw new InvalidOperationException("Non-whitelisted AccessKit test key refused");
        if (hwnd == IntPtr.Zero || Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Invalid target or Windows x64 INPUT layout");
        uint owner;
        uint thread = GetWindowThreadProcessId(hwnd, out owner);
        if (thread == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("AccessKit key target PID mismatch");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing AccessKit key input to a nonforeground window");
        var events = new Input[shift ? 4 : 2];
        int i = 0;
        if (shift)
            events[i++] = new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_SHIFT };
        events[i++] = new Input { Type = INPUT_KEYBOARD, VirtualKey = key };
        events[i++] = new Input { Type = INPUT_KEYBOARD, VirtualKey = key, Flags = KEYEVENTF_KEYUP };
        if (shift)
            events[i++] = new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_SHIFT, Flags = KEYEVENTF_KEYUP };
        uint sent = SendInput((uint)events.Length, events, Marshal.SizeOf(typeof(Input)));
        if (sent != events.Length)
            throw new InvalidOperationException("AccessKit SendInput inserted " + sent + "/" + events.Length
                + " events; Win32 error " + Marshal.GetLastWin32Error());
    }

    public static void TypeAsciiKeys(IntPtr hwnd, int expectedPid, string text) {
        if (hwnd == IntPtr.Zero || String.IsNullOrEmpty(text))
            throw new InvalidOperationException("Missing window or synthetic text");
        if (Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Unexpected x64 Windows INPUT size");
        uint owner;
        uint targetThread = GetWindowThreadProcessId(hwnd, out owner);
        if (targetThread == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("Window PID does not match launched Akula");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing to type into a nonforeground window");
        if ((GetKeyState(0x14) & 1) != 0)
            throw new InvalidOperationException("Caps Lock must be off for exact ASCII marker");
        IntPtr layout = GetKeyboardLayout(targetThread);
        if (layout == IntPtr.Zero)
            throw new InvalidOperationException("Foreground keyboard layout unavailable");

        const uint INPUT_KEYBOARD = 1;
        const uint KEYEVENTF_KEYUP = 0x0002;
        const ushort VK_SHIFT = 0x10;
        var events = new System.Collections.Generic.List<Input>(text.Length * 4);
        foreach (char ch in text) {
            // No general-purpose system typing interface: the release smoke is
            // restricted to its own generated ASCII marker and no user content.
            bool allowed = (ch >= 'A' && ch <= 'Z')
                || (ch >= 'a' && ch <= 'z')
                || (ch >= '0' && ch <= '9') || ch == '_';
            if (!allowed)
                throw new InvalidOperationException("Non-ASCII synthetic marker refused");
            short mapping = VkKeyScanExW(ch, layout);
            if (mapping == -1)
                throw new InvalidOperationException("Marker char is unavailable on runner layout");
            int modifiers = (mapping >> 8) & 0xff;
            if ((modifiers & ~1) != 0)
                throw new InvalidOperationException("Unexpected Ctrl/Alt modifier for marker");
            ushort vk = (ushort)(mapping & 0xff);
            ushort scan = (ushort)MapVirtualKeyExW(vk, 0, layout);
            if (scan == 0)
                throw new InvalidOperationException("Missing virtual-key scan code on runner");
            if ((modifiers & 1) != 0)
                events.Add(new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_SHIFT });
            events.Add(new Input { Type = INPUT_KEYBOARD, VirtualKey = vk, ScanCode = scan });
            events.Add(new Input { Type = INPUT_KEYBOARD, VirtualKey = vk, ScanCode = scan, Flags = KEYEVENTF_KEYUP });
            if ((modifiers & 1) != 0)
                events.Add(new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_SHIFT, Flags = KEYEVENTF_KEYUP });
        }
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Foreground window changed before OS key injection");
        uint sent = SendInput((uint)events.Count, events.ToArray(), Marshal.SizeOf(typeof(Input)));
        if (sent != (uint)events.Count)
            throw new InvalidOperationException("Physical-key SendInput injected " + sent + "/" + events.Count
                + " events; Win32 error " + Marshal.GetLastWin32Error());
    }

    // Native OS keyboard chords for read-only GUI model acceptance after
    // restart. Only Ctrl+A (select all) and Ctrl+C (copy) are allowed.
    public static void PressCtrlShortcut(IntPtr hwnd, int expectedPid, ushort key) {
        if (key != 0x41 && key != 0x43)
            throw new InvalidOperationException("Only selection/copy shortcuts are authorized");
        if (hwnd == IntPtr.Zero || Marshal.SizeOf(typeof(Input)) != 40)
            throw new InvalidOperationException("Unexpected Windows shortcut window/input layout");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("Shortcut window PID does not match Akula");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing shortcut on nonforeground window");

        const uint INPUT_KEYBOARD = 1;
        const uint KEYEVENTF_KEYUP = 0x0002;
        const ushort VK_CONTROL = 0x11;
        var keys = new Input[] {
            new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_CONTROL },
            new Input { Type = INPUT_KEYBOARD, VirtualKey = key },
            new Input { Type = INPUT_KEYBOARD, VirtualKey = key, Flags = KEYEVENTF_KEYUP },
            new Input { Type = INPUT_KEYBOARD, VirtualKey = VK_CONTROL, Flags = KEYEVENTF_KEYUP }
        };
        uint sent = SendInput((uint)keys.Length, keys, Marshal.SizeOf(typeof(Input)));
        if (sent != (uint)keys.Length)
            throw new InvalidOperationException("Native Ctrl shortcut wrote " + sent + "/" + keys.Length
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
  # --focus targets the real editor; inject layout-mapped virtual keys.
  [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $Process.Id, $Text)
}

function Send-AkulaBetaShortcut {
  param(
    [Parameter(Mandatory)][System.Diagnostics.Process]$Process,
    [Parameter(Mandatory)][ValidateSet('SelectAll', 'Copy')][string]$Action
  )
  $Process.Refresh()
  if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
    throw 'Native shortcut target has no installed Akula window'
  }
  if ($Process.MainWindowTitle -notlike 'First note*' -or
      $Process.MainWindowTitle -notlike '*Akula*') {
    throw 'Native shortcut target is not the reopened synthetic note'
  }
  $hwnd = [IntPtr]$Process.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
  $focused = $false
  for ($attempt = 0; $attempt -lt 15; $attempt++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 250
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
      $focused = $true
      break
    }
  }
  if (!$focused) { throw 'Cannot focus reopened Akula before native Ctrl shortcut' }
  $key = if ($Action -eq 'SelectAll') { [ushort]0x41 } else { [ushort]0x43 }
  [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $Process.Id, $key)
}
