using System.ComponentModel;
using System.Diagnostics;
using Protocol = Aiwin11.V1;

/// Read-only process snapshot for the agent's `list_processes` tool: the processes using the
/// most memory, with name, ID, working set, total CPU time and whether they own a window.
/// Never opens a process for writing, never stops anything.
static class ProcessLister
{
    public const uint MaxLimit = 25;
    const int MaxName = 64;

    public static Protocol.ProcessList Read(uint limit)
    {
        var entries = new List<Protocol.ProcessEntry>();
        uint total = 0;
        foreach (var process in Process.GetProcesses())
        {
            using (process)
            {
                try
                {
                    // The System Idle Process (ID 0) is not a real process.
                    if (process.Id == 0) continue;
                    total++;
                    var name = process.ProcessName;
                    if (string.IsNullOrEmpty(name)) continue;
                    if (name.Length > MaxName) name = name[..MaxName];
                    ulong cpu = 0;
                    try
                    {
                        cpu = (ulong)process.TotalProcessorTime.TotalMilliseconds;
                    }
                    catch (Exception error) when (error is Win32Exception or InvalidOperationException or NotSupportedException)
                    {
                        // Protected process: CPU time is not readable by a standard user.
                    }
                    bool window = false;
                    try
                    {
                        window = process.MainWindowHandle != IntPtr.Zero;
                    }
                    catch (InvalidOperationException)
                    {
                    }
                    entries.Add(new Protocol.ProcessEntry
                    {
                        ProcessId = (uint)process.Id,
                        Name = name,
                        WorkingSetBytes = (ulong)Math.Max(0, process.WorkingSet64),
                        CpuTimeMs = cpu,
                        HasWindow = window,
                    });
                }
                catch (InvalidOperationException)
                {
                    // The process exited while being read.
                }
            }
        }
        var list = new Protocol.ProcessList { Total = total };
        list.Processes.AddRange(entries.OrderByDescending(entry => entry.WorkingSetBytes)
            .Take((int)Math.Min(limit, MaxLimit)));
        return list;
    }

    /// <summary>Read-only details of one process; null if it does not exist (any more).</summary>
    public static Protocol.ProcessDetails? Inspect(uint processId)
    {
        if (processId == 0) return null;
        Process process;
        try
        {
            process = Process.GetProcessById((int)processId);
        }
        catch (ArgumentException)
        {
            return null;
        }
        using (process)
        {
            try
            {
                var name = process.ProcessName;
                if (string.IsNullOrEmpty(name)) return null;
                var details = new Protocol.ProcessDetails
                {
                    ProcessId = processId,
                    Name = name.Length <= MaxName ? name : name[..MaxName],
                    WorkingSetBytes = (ulong)Math.Max(0, process.WorkingSet64),
                    PrivateBytes = (ulong)Math.Max(0, process.PrivateMemorySize64),
                    Threads = (uint)process.Threads.Count,
                    Handles = (uint)Math.Max(0, process.HandleCount),
                };
                // Each of these may be refused for a protected or elevated process: left empty.
                Try(() => details.CpuTimeMs = (ulong)process.TotalProcessorTime.TotalMilliseconds);
                Try(() => details.StartTimeUnixMs = new DateTimeOffset(process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds());
                Try(() => details.HasWindow = process.MainWindowHandle != IntPtr.Zero);
                Try(() =>
                {
                    var path = process.MainModule?.FileName ?? "";
                    if (path.Length <= 260) details.ExecutablePath = path;
                    if (path.Length > 0)
                    {
                        var version = FileVersionInfo.GetVersionInfo(path);
                        details.Description = Bounded(version.FileDescription);
                        details.Company = Bounded(version.CompanyName);
                    }
                });
                Try(() => details.ParentProcessId = ParentOf(processId));
                return details;
            }
            catch (InvalidOperationException)
            {
                return null; // Exited while being read.
            }
        }
    }

    static string Bounded(string? text)
    {
        var value = (text ?? "").Trim();
        return value.Length <= 120 && !value.Any(char.IsControl) ? value : "";
    }

    static uint ParentOf(uint processId)
    {
        using var searcher = new System.Management.ManagementObjectSearcher(
            $"SELECT ParentProcessId FROM Win32_Process WHERE ProcessId = {processId}");
        foreach (var item in searcher.Get())
            using (item)
                return item["ParentProcessId"] is uint parent ? parent : 0;
        return 0;
    }

    static void Try(Action read)
    {
        try
        {
            read();
        }
        catch (Exception error) when (error is Win32Exception or InvalidOperationException or NotSupportedException
            or UnauthorizedAccessException or System.Management.ManagementException or System.IO.IOException)
        {
        }
    }

    /// <summary>Processes never stopped, whatever the request (checked again by the host).</summary>
    static readonly HashSet<string> Protected = new(StringComparer.OrdinalIgnoreCase) { "system", "registry", "smss", "csrss", "wininit", "winlogon", "services", "lsass", "lsaiso", "svchost", "fontdrvhost", "dwm", "explorer", "sihost", "ctfmon", "taskhostw", "conhost", "audiodg", "spoolsv", "searchhost", "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "securityhealthsystray", "securityhealthservice", "msmpeng", "nissrv", "mssense", "wudfhost", "lockapp", "userinit", "aiwin11-desktop", "aiwin11.windows", "aiwin11.admin", "msedgewebview2" };

    /// <summary>
    /// Stops one process: "closed" (asked to close and exited), "ended" (no window, ended),
    /// "still_running" (asked to close, still open: never forced), or an error code.
    /// </summary>
    public static string Stop(uint processId, string expectedName)
    {
        if (processId <= 4 || processId == (uint)Environment.ProcessId) return "protected_process";
        Process process;
        try
        {
            process = Process.GetProcessById((int)processId);
        }
        catch (ArgumentException)
        {
            return "process_unavailable";
        }
        using (process)
        {
            try
            {
                if (!string.Equals(process.ProcessName, expectedName, StringComparison.OrdinalIgnoreCase))
                    return "process_changed";
                if (Protected.Contains(process.ProcessName)
                    || process.SessionId != Process.GetCurrentProcess().SessionId)
                    return "protected_process";
                if (process.MainWindowHandle != IntPtr.Zero)
                {
                    process.CloseMainWindow();
                    return process.WaitForExit(5000) ? "closed" : "still_running";
                }
                process.Kill(entireProcessTree: false);
                return process.WaitForExit(5000) ? "ended" : "still_running";
            }
            catch (Exception error) when (error is Win32Exception or InvalidOperationException or NotSupportedException)
            {
                // Access denied (elevated or protected process) or already exited.
                return process.HasExited ? "ended" : "access_denied";
            }
        }
    }
}
