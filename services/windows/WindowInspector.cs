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

    private const int MaxListed = 20;

    /// <summary>Topmost visible, unowned, uncloaked application window not owned by the host.</summary>
    internal static Protocol.WindowInfo? Locate(uint excludeProcessId)
    {
        for (nint window = GetTopWindow(0); window != 0; window = GetWindow(window, GwHwndNext))
        {
            if (Describe(window, excludeProcessId, allowMinimized: false) is { } info)
                return info;
        }
        return null;
    }

    /// <summary>Application windows in Z order (minimized ones included), at most 20.</summary>
    internal static Protocol.WindowList List(uint excludeProcessId)
    {
        var list = new Protocol.WindowList();
        for (nint window = GetTopWindow(0); window != 0 && list.Windows.Count < MaxListed; window = GetWindow(window, GwHwndNext))
        {
            if (Describe(window, excludeProcessId, allowMinimized: true) is { } info)
                list.Windows.Add(info);
        }
        return list;
    }

    /// <summary>
    /// Applies one arrangement to this exact window if it still belongs to the process, then
    /// checks the resulting state. Null if the window changed hands or disappeared.
    /// </summary>
    internal static bool? Arrange(ulong handle, uint processId, Protocol.ArrangeWindow.Types.Arrangement arrangement)
    {
        nint window = (nint)handle;
        if (!OwnedBy(window, processId) || !IsWindowVisible(window))
            return null;
        switch (arrangement)
        {
            case Protocol.ArrangeWindow.Types.Arrangement.Minimize:
                ShowWindow(window, SwMinimize);
                return Settled(() => IsIconic(window));
            case Protocol.ArrangeWindow.Types.Arrangement.Maximize:
                ShowWindow(window, SwMaximize);
                return Settled(() => IsZoomed(window));
            case Protocol.ArrangeWindow.Types.Arrangement.Restore:
                ShowWindow(window, SwRestore);
                return Settled(() => !IsIconic(window) && !IsZoomed(window));
            case Protocol.ArrangeWindow.Types.Arrangement.Focus:
                if (IsIconic(window))
                    ShowWindow(window, SwRestore);
                // Windows may refuse to change the foreground window; this is then reported.
                SetForegroundWindow(window);
                return Settled(() => GetForegroundWindow() == window);
            default:
                return null;
        }
    }

    /// <summary>Waits up to one second for a window state change to be visible.</summary>
    private static bool Settled(Func<bool> done)
    {
        var watch = Stopwatch.StartNew();
        while (watch.ElapsedMilliseconds < 1000)
        {
            if (done())
                return true;
            Thread.Sleep(50);
        }
        return done();
    }

    private static Protocol.WindowInfo? Describe(nint window, uint excludeProcessId, bool allowMinimized)
    {
        uint self = (uint)Environment.ProcessId;
        bool minimized = IsIconic(window);
        if (!IsWindowVisible(window) || (minimized && !allowMinimized) || GetWindow(window, GwOwner) != 0)
            return null;
        long extended = GetWindowLongPtrW(window, GwlExStyle);
        if ((extended & (WsExTopmost | WsExToolWindow | WsExNoActivate)) != 0)
            return null;
        if (DwmGetWindowAttribute(window, DwmwaCloaked, out int cloaked, sizeof(int)) == 0 && cloaked != 0)
            return null;
        if (GetWindowThreadProcessId(window, out uint pid) == 0 || pid == 0 || pid == excludeProcessId || pid == self)
            return null;
        if (ShellClasses.Contains(ClassName(window)) || !GetWindowRect(window, out var rect)
            || (!minimized && (rect.Right <= rect.Left || rect.Bottom <= rect.Top)))
            return null;
        string title = Clean(Text(window), MaxTitle);
        if (title.Length == 0)
            return null;
        string processName;
        try
        {
            using var process = Process.GetProcessById((int)pid);
            processName = Clean(process.ProcessName, 64);
        }
        catch (Exception error) when (error is ArgumentException or InvalidOperationException)
        {
            return null;
        }
        if (processName.Length == 0)
            return null;
        return new Protocol.WindowInfo
        {
            Window = (ulong)window, ProcessId = pid, Title = title, ProcessName = processName,
            Minimized = minimized && allowMinimized,
        };
    }

    /// <summary>Named control tree of this exact window, or null if it changed hands.</summary>
    internal static Protocol.UiTree? Inspect(ulong handle, uint processId, long deadline) =>
        Inspect(handle, processId, deadline, null);

    /// <summary>
    /// Invokes control number `index` of the same walk as Inspect, if its type and name still
    /// match. Null: window or control gone or changed; false: the control cannot be invoked.
    /// </summary>
    internal static bool? Invoke(ulong handle, uint processId, uint index, uint controlType, string name, long deadline,
        string? value = null)
    {
        var elements = new List<AutomationElement>();
        var tree = Inspect(handle, processId, deadline, elements);
        if (tree is null || index == 0 || index > elements.Count)
            return null;
        var node = tree.Nodes[(int)index - 1];
        if (node.ControlType != controlType || node.Name != name)
            return null;
        var element = elements[(int)index - 1];
        if (!element.Current.IsEnabled || element.Current.IsPassword)
            return false;
        if (value is not null)
        {
            if (!element.TryGetCurrentPattern(ValuePattern.Pattern, out var pattern)
                || ((ValuePattern)pattern).Current.IsReadOnly)
                return false;
            ((ValuePattern)pattern).SetValue(value);
            return ((ValuePattern)pattern).Current.Value == value;
        }
        if (element.TryGetCurrentPattern(InvokePattern.Pattern, out var invoke))
        {
            ((InvokePattern)invoke).Invoke();
            return true;
        }
        if (element.TryGetCurrentPattern(TogglePattern.Pattern, out var toggle))
        {
            ((TogglePattern)toggle).Toggle();
            return true;
        }
        if (element.TryGetCurrentPattern(SelectionItemPattern.Pattern, out var select))
        {
            ((SelectionItemPattern)select).Select();
            return true;
        }
        return false;
    }

    private static Protocol.UiTree? Inspect(ulong handle, uint processId, long deadline, List<AutomationElement>? elements)
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

            elements?.Add(element);
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
    [DllImport("user32.dll")] private static extern bool IsZoomed(nint window);
    [DllImport("user32.dll")] private static extern bool ShowWindow(nint window, int command);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(nint window);
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    private const int SwMaximize = 3;
    private const int SwMinimize = 6;
    private const int SwRestore = 9;
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
