using System.Runtime.InteropServices;
using System.Text;

/// Input directed to a previously resolved Win32 control without changing global focus.
internal static class ControlInput
{
    private const uint ButtonClick = 0x00F5;
    private const uint Character = 0x0102;
    private const uint GetText = 0x000D;
    private const uint GetTextLength = 0x000E;
    private const uint TimeoutFlags = 0x0002 | 0x0020;
    private const int EditPasswordOrReadOnly = 0x0020 | 0x0800;

    internal static WindowInspector.ControlResult Apply(
        nint root, uint processId, nint child, uint controlType, string? value, long deadline)
    {
        bool StillTarget() => child != 0 && IsChild(root, child) && IsWindowVisible(child)
            && IsWindowEnabled(child) && GetWindowThreadProcessId(child, out uint owner) != 0
            && owner == processId && GetWindowThreadProcessId(root, out uint rootOwner) != 0
            && rootOwner == processId && DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() < deadline;

        if (!StillTarget()) return WindowInspector.ControlResult.Unavailable;
        var name = new StringBuilder(32);
        if (GetClassNameW(child, name, name.Capacity) == 0)
            return WindowInspector.ControlResult.Unavailable;

        uint Remaining() => (uint)Math.Clamp(deadline - DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), 0, 250);
        bool Send(uint message, nuint parameter, out nuint result) =>
            SendMessageTimeoutW(child, message, parameter, 0, TimeoutFlags, Remaining(), out result) != 0;

        if (value is null && controlType == 50000 && name.ToString() == "Button" && StillTarget())
        {
            // Delivery does not prove that the application performed its action.
            Send(ButtonClick, 0, out _);
            return WindowInspector.ControlResult.InputUnverified;
        }
        if (value is null || controlType != 50004 || name.ToString() != "Edit"
            || value.Length is < 1 or > 64 || value.Any(char.IsControl) || !IsWindowUnicode(child))
            return WindowInspector.ControlResult.Unavailable;

        bool CanType() => StillTarget() && (GetWindowLongW(child, -16) & EditPasswordOrReadOnly) == 0;
        if (!CanType() || !Send(GetTextLength, 0, out nuint length) || length != 0)
            return WindowInspector.ControlResult.Unavailable;

        foreach (char character in value)
        {
            if (!CanType() || !Send(Character, character, out _))
                return WindowInspector.ControlResult.InputUnverified;
        }
        var actual = new StringBuilder(value.Length + 2);
        return CanType()
            && SendTextMessage(child, GetText, (nuint)actual.Capacity, actual, TimeoutFlags, Remaining(), out _) != 0
            && actual.ToString() == value
            ? WindowInspector.ControlResult.KeyboardVerified
            : WindowInspector.ControlResult.InputUnverified;
    }

    [DllImport("user32.dll")] private static extern bool IsChild(nint parent, nint child);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint window);
    [DllImport("user32.dll")] private static extern bool IsWindowEnabled(nint window);
    [DllImport("user32.dll")] private static extern bool IsWindowUnicode(nint window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
    [DllImport("user32.dll", ExactSpelling = true)] private static extern int GetWindowLongW(nint window, int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetClassNameW(nint window, StringBuilder name, int capacity);
    [DllImport("user32.dll", ExactSpelling = true)]
    private static extern nint SendMessageTimeoutW(nint window, uint message, nuint wParam, nint lParam,
        uint flags, uint timeout, out nuint result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern nint SendTextMessage(nint window, uint message, nuint wParam, StringBuilder lParam,
        uint flags, uint timeout, out nuint result);
}
