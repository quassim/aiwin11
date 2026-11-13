using System.Runtime.InteropServices;
using Protocol = Aiwin11.V1;

/// Sends one allowlisted key or Ctrl+key shortcut to a listed, verified window.
internal static class KeyInput
{
    private const uint InputKeyboard = 1;
    private const uint KeyeventfKeyup = 0x0002;
    private const ushort VkControl = 0x11;

    internal static string Apply(ulong window, uint processId, Protocol.SendKey.Types.Key key, long deadlineMs)
    {
        var (ctrl, vk) = MapKey(key);
        if (vk == 0) return "invalid_key";

        var hwnd = (nint)window;
        if (hwnd == 0 || !IsWindow(hwnd) || !IsWindowVisible(hwnd) || !IsWindowEnabled(hwnd))
            return "window_unavailable";
        if (GetWindowThreadProcessId(hwnd, out uint owner) == 0 || owner != processId)
            return "window_changed";
        if (IsProcessElevated(owner))
            return "window_elevated";
        if (DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadlineMs)
            return "expired";

        if (GetForegroundWindow() != hwnd)
        {
            SetForegroundWindow(hwnd);
            Thread.Sleep(50);
        }
        if (GetForegroundWindow() != hwnd)
            return "focus_unavailable";

        var inputs = new List<INPUT>(4);
        if (ctrl) inputs.Add(KeyEvent(VkControl, false));
        inputs.Add(KeyEvent(vk, false));
        inputs.Add(KeyEvent(vk, true));
        if (ctrl) inputs.Add(KeyEvent(VkControl, true));

        uint sent = SendInput((uint)inputs.Count, inputs.ToArray(), Marshal.SizeOf<INPUT>());
        return sent == (uint)inputs.Count ? "ok" : "input_failed";
    }

    static (bool Ctrl, ushort Vk) MapKey(Protocol.SendKey.Types.Key key) => key switch
    {
        Protocol.SendKey.Types.Key.Enter => (false, 0x0D),
        Protocol.SendKey.Types.Key.Escape => (false, 0x1B),
        Protocol.SendKey.Types.Key.Tab => (false, 0x09),
        Protocol.SendKey.Types.Key.Space => (false, 0x20),
        Protocol.SendKey.Types.Key.Up => (false, 0x26),
        Protocol.SendKey.Types.Key.Down => (false, 0x28),
        Protocol.SendKey.Types.Key.Left => (false, 0x25),
        Protocol.SendKey.Types.Key.Right => (false, 0x27),
        Protocol.SendKey.Types.Key.CtrlC => (true, 0x43),
        Protocol.SendKey.Types.Key.CtrlV => (true, 0x56),
        Protocol.SendKey.Types.Key.CtrlA => (true, 0x41),
        Protocol.SendKey.Types.Key.CtrlZ => (true, 0x5A),
        Protocol.SendKey.Types.Key.CtrlS => (true, 0x53),
        Protocol.SendKey.Types.Key.CtrlF => (true, 0x46),
        _ => (false, (ushort)0),
    };

    static INPUT KeyEvent(ushort vk, bool keyUp) => new()
    {
        type = InputKeyboard,
        u = new InputUnion
        {
            ki = new KEYBDINPUT
            {
                wVk = vk,
                dwFlags = keyUp ? KeyeventfKeyup : 0,
            },
        },
    };

    [StructLayout(LayoutKind.Sequential)]
    struct INPUT
    {
        public uint type;
        public InputUnion u;
    }

    [StructLayout(LayoutKind.Explicit)]
    struct InputUnion
    {
        [FieldOffset(0)] public MOUSEINPUT mi;
        [FieldOffset(0)] public KEYBDINPUT ki;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT
    {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint dwFlags;
        public uint time;
        public nuint dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct KEYBDINPUT
    {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public nuint dwExtraInfo;
    }

    [DllImport("user32.dll")] private static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint window);
    [DllImport("user32.dll")] private static extern bool IsWindowEnabled(nint window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(nint window);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, INPUT[] inputs, int size);

    static bool IsProcessElevated(uint processId)
    {
        const uint ProcessQueryLimitedInformation = 0x1000;
        const uint TokenQuery = 0x0008;
        const int TokenElevationClass = 20;
        nint process = OpenProcess(ProcessQueryLimitedInformation, false, processId);
        if (process == 0)
            return Marshal.GetLastWin32Error() == 5;
        try
        {
            if (!OpenProcessToken(process, TokenQuery, out nint token))
                return Marshal.GetLastWin32Error() == 5;
            try
            {
                int size = Marshal.SizeOf<int>();
                nint ptr = Marshal.AllocHGlobal(size);
                try
                {
                    if (GetTokenInformation(token, TokenElevationClass, ptr, size, out _))
                        return Marshal.ReadInt32(ptr) != 0;
                    return false;
                }
                finally { Marshal.FreeHGlobal(ptr); }
            }
            finally { CloseHandle(token); }
        }
        finally { CloseHandle(process); }
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern nint OpenProcess(uint dwDesiredAccess, bool bInheritHandle, uint dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(nint hObject);

    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool OpenProcessToken(nint processHandle, uint desiredAccess, out nint tokenHandle);

    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool GetTokenInformation(
        nint tokenHandle, int tokenInformationClass, nint tokenInformation, int tokenInformationLength, out int returnLength);
}
