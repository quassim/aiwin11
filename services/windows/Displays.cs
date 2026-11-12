using System.Runtime.InteropServices;
using Protocol = Aiwin11.V1;

internal static class Displays
{
    private const int MaxDisplays = 32;
    private const uint PrimaryMonitor = 1;
    private static readonly nint PerMonitorAwareV2 = new(-4);

    internal static Protocol.DisplayList? Read()
    {
        nint previousContext = SetThreadDpiAwarenessContext(PerMonitorAwareV2);
        if (previousContext == 0)
            return null;

        try
        {
            var monitors = new List<Protocol.DisplayInfo>();
            bool complete = EnumDisplayMonitors(0, 0, (monitor, _, _, _) =>
            {
                if (monitors.Count == MaxDisplays)
                    return false;
                var info = new MonitorInfo { Size = (uint)Marshal.SizeOf<MonitorInfo>() };
                if (!GetMonitorInfo(monitor, ref info))
                    return false;
                long width = (long)info.Bounds.Right - info.Bounds.Left;
                long height = (long)info.Bounds.Bottom - info.Bounds.Top;
                if (width is <= 0 or > int.MaxValue || height is <= 0 or > int.MaxValue)
                    return false;
                monitors.Add(new Protocol.DisplayInfo
                {
                    X = info.Bounds.Left,
                    Y = info.Bounds.Top,
                    Width = (uint)width,
                    Height = (uint)height,
                    Primary = (info.Flags & PrimaryMonitor) != 0,
                });
                return true;
            }, 0);
            if (!complete || monitors.Count(monitor => monitor.Primary) != 1)
                return null;
            var result = new Protocol.DisplayList();
            result.Displays.AddRange(monitors.OrderByDescending(monitor => monitor.Primary)
                .ThenBy(monitor => monitor.Y).ThenBy(monitor => monitor.X));
            return result;
        }
        finally
        {
            SetThreadDpiAwarenessContext(previousContext);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect
    {
        public int Left, Top, Right, Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo
    {
        public uint Size;
        public Rect Bounds, WorkArea;
        public uint Flags;
    }

    private delegate bool MonitorCallback(nint monitor, nint context, nint bounds, nint data);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool EnumDisplayMonitors(nint context, nint clip, MonitorCallback callback, nint data);

    [DllImport("user32.dll", EntryPoint = "GetMonitorInfoW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetMonitorInfo(nint monitor, ref MonitorInfo info);

    [DllImport("user32.dll")]
    private static extern nint SetThreadDpiAwarenessContext(nint context);
}
