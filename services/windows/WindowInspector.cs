using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Automation;
using Protocol = Aiwin11.V1;

/// <summary>
/// Agent-facing window reading. Locate finds the window the user was using before the host;
/// Inspect reads the named UI Automation tree of exactly that window after approval.
/// </summary>
internal static class WindowInspector
{
    private const int MaxNodes = 80;
    private const int MaxDepth = 8;
    private const int MaxName = 80;
    private const int MaxTitle = 120;

    private const int GwOwner = 4;
    private const int GwHwndNext = 2;
    private const int GwlExStyle = -20;
    private const long WsExTopmost = 0x00000008;
    private const long WsExToolWindow = 0x00000080;
    private const long WsExNoActivate = 0x08000000;
    private const int DwmwaCloaked = 14;
    private static readonly HashSet<string> ShellClasses = new(StringComparer.Ordinal)
    {
        "Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd",
    };

    /// <summary>Topmost visible, unowned, uncloaked application window not owned by the host.</summary>
    internal static Protocol.WindowInfo? Locate(uint excludeProcessId)
    {
        uint self = (uint)Environment.ProcessId;
        for (nint window = GetTopWindow(0); window != 0; window = GetWindow(window, GwHwndNext))
        {
            if (!IsWindowVisible(window) || IsIconic(window) || GetWindow(window, GwOwner) != 0)
                continue;
            long extended = GetWindowLongPtrW(window, GwlExStyle);
            if ((extended & (WsExTopmost | WsExToolWindow | WsExNoActivate)) != 0)
                continue;
            if (DwmGetWindowAttribute(window, DwmwaCloaked, out int cloaked, sizeof(int)) == 0 && cloaked != 0)
                continue;
            if (GetWindowThreadProcessId(window, out uint pid) == 0 || pid == 0 || pid == excludeProcessId || pid == self)
                continue;
            if (ShellClasses.Contains(ClassName(window)) || !GetWindowRect(window, out var rect)
                || rect.Right <= rect.Left || rect.Bottom <= rect.Top)
                continue;
            string title = Clean(Text(window), MaxTitle);
            if (title.Length == 0)
                continue;
            string processName;
            try
            {
                using var process = Process.GetProcessById((int)pid);
                processName = Clean(process.ProcessName, 64);
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException)
            {
                continue;
            }
            return new Protocol.WindowInfo
            {
                Window = (ulong)window, ProcessId = pid, Title = title, ProcessName = processName,
            };
        }
        return null;
    }

    /// <summary>Named control tree of this exact window, or null if it changed hands.</summary>
    internal static Protocol.UiTree? Inspect(ulong handle, uint processId, long deadline)
    {
        nint window = (nint)handle;
        if (!OwnedBy(window, processId))
            return null;

        var root = AutomationElement.FromHandle(window);
        var tree = new Protocol.UiTree();
        Walk(root, 0, 0);
        // A recycled handle or a process swap during the walk invalidates the result.
        return tree.Nodes.Count > 0 && OwnedBy(window, processId) ? tree : null;

        void Walk(AutomationElement element, uint parent, int depth)
        {
            if (tree.Nodes.Count == MaxNodes || depth > MaxDepth
                || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline)
            {
                tree.Truncated = true;
                return;
            }
            var current = element.Current;
            if (current.ProcessId != processId)
                return;

            tree.Nodes.Add(new Protocol.UiNode
            {
                Parent = parent,
                ControlType = (uint)current.ControlType.Id,
                Enabled = current.IsEnabled,
                Focused = current.HasKeyboardFocus,
                // Password fields never expose a name, whatever the application puts there.
                Name = current.IsPassword ? "" : Clean(current.Name, MaxName),
            });
            uint index = (uint)tree.Nodes.Count;
            var walker = TreeWalker.ControlViewWalker;
            for (var child = walker.GetFirstChild(element); child is not null; child = walker.GetNextSibling(child))
            {
                Walk(child, index, depth + 1);
                if (tree.Nodes.Count == MaxNodes || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline)
                {
                    tree.Truncated = true;
                    break;
                }
                if (depth == MaxDepth)
                    break;
            }
        }
    }

    private static bool OwnedBy(nint window, uint processId) =>
        window != 0 && IsWindow(window) && GetWindowThreadProcessId(window, out uint pid) != 0 && pid == processId;

    private static string Text(nint window)
    {
        var buffer = new StringBuilder(MaxTitle + 2);
        GetWindowTextW(window, buffer, buffer.Capacity);
        return buffer.ToString();
    }

    private static string ClassName(nint window)
    {
        var buffer = new StringBuilder(64);
        return GetClassNameW(window, buffer, buffer.Capacity) > 0 ? buffer.ToString() : "";
    }

    /// <summary>Single line, no control characters, trimmed and bounded.</summary>
    private static string Clean(string? value, int max)
    {
        if (string.IsNullOrEmpty(value))
            return "";
        var builder = new StringBuilder(Math.Min(value.Length, max));
        foreach (char character in value)
        {
            if (builder.Length == max)
                break;
            builder.Append(char.IsControl(character) ? ' ' : character);
        }
        return builder.ToString().Trim();
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] private static extern nint GetTopWindow(nint window);
    [DllImport("user32.dll")] private static extern nint GetWindow(nint window, int command);
    [DllImport("user32.dll")] private static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint window);
    [DllImport("user32.dll")] private static extern bool IsIconic(nint window);
    [DllImport("user32.dll")] private static extern long GetWindowLongPtrW(nint window, int index);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(nint window, out Rect rect);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetWindowTextW(nint window, StringBuilder text, int maxCount);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetClassNameW(nint window, StringBuilder className, int maxCount);
    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(nint window, int attribute, out int value, int size);
}
