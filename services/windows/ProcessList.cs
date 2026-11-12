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
}
