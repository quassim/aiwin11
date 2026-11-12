using System.IO;
using System.ComponentModel;
using System.Diagnostics;
using Protocol = Aiwin11.V1;

/// Closed list of applications other than Notepad that the agent may open: fixed System32
/// executables, no arguments. Success means a new window of the expected process appeared.
internal static class AppLauncher
{
    internal static (string Path, string ProcessName)? Target(Protocol.LaunchApplication.Types.Application application) =>
        application switch
        {
            Protocol.LaunchApplication.Types.Application.CharacterMap =>
                (Path.Combine(Environment.SystemDirectory, "charmap.exe"), "charmap"),
            Protocol.LaunchApplication.Types.Application.Paint =>
                (Path.Combine(Environment.SystemDirectory, "mspaint.exe"), "mspaint"),
            _ => ((string, string)?)null,
        };

    /// Returns the process ID of the new window, or null if none was verified in time.
    internal static uint? Launch(string path, string processName, long deadlineMs)
    {
        var existing = Windows(processName);
        Notepad.StartExecutable(path);
        var watch = Stopwatch.StartNew();
        while (watch.Elapsed < TimeSpan.FromSeconds(6) && DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() < deadlineMs)
        {
            var opened = Windows(processName).Except(existing).FirstOrDefault();
            if (opened != default) return (uint)opened.Pid;
            Thread.Sleep(100);
        }
        return null;
    }

    static HashSet<(int Pid, nint Window)> Windows(string processName)
    {
        var windows = new HashSet<(int, nint)>();
        foreach (var process in Process.GetProcessesByName(processName))
        {
            using (process)
            {
                try
                {
                    if (process.MainWindowHandle != 0) windows.Add((process.Id, process.MainWindowHandle));
                }
                catch (Exception error) when (error is Win32Exception or InvalidOperationException)
                {
                }
            }
        }
        return windows;
    }
}
