using System.IO;
using System.Runtime.InteropServices;
using Protocol = Aiwin11.V1;

/// Read-only system metrics for the agent's `get_system_status` tool: physical memory,
/// processor load sampled over 250 ms, uptime and fixed drives' space. Standard-user APIs only.
static class SystemStatusReader
{
    const int MaxDrives = 8;

    public static Protocol.SystemStatus? Read(long deadlineMs)
    {
        var memory = new MemoryStatusEx { Length = (uint)Marshal.SizeOf<MemoryStatusEx>() };
        if (!GlobalMemoryStatusEx(ref memory)) return null;
        if (!GetSystemTimes(out var idle1, out var kernel1, out var user1)) return null;
        Thread.Sleep(250);
        if (DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadlineMs) return null;
        if (!GetSystemTimes(out var idle2, out var kernel2, out var user2)) return null;
        // Kernel time includes idle time.
        ulong idle = idle2 - idle1;
        ulong total = (kernel2 - kernel1) + (user2 - user1);
        uint load = total == 0 ? 0 : (uint)Math.Clamp(100.0 * (total - Math.Min(idle, total)) / total, 0, 100);

        var status = new Protocol.SystemStatus
        {
            MemoryTotalBytes = memory.TotalPhys,
            MemoryAvailableBytes = memory.AvailPhys,
            CpuLoadPercent = load,
            LogicalProcessors = (uint)Environment.ProcessorCount,
            UptimeSeconds = (ulong)(Environment.TickCount64 / 1000),
        };
        try
        {
            using var searcher = new System.Management.ManagementObjectSearcher(
                "SELECT Name, DriverVersion, AdapterRAM FROM Win32_VideoController");
            foreach (var adapter in searcher.Get())
            {
                using (adapter)
                {
                    if (status.Gpus.Count >= 4) break;
                    var name = (adapter["Name"] as string ?? "").Trim();
                    if (name.Length == 0) continue;
                    status.Gpus.Add(new Protocol.GpuInfo
                    {
                        Name = name.Length <= 120 ? name : name[..120],
                        DriverVersion = (adapter["DriverVersion"] as string ?? "").Trim() is var v && v.Length <= 40 ? v : "",
                        MemoryBytes = adapter["AdapterRAM"] is uint ram ? ram : 0,
                    });
                }
            }
        }
        catch (Exception error) when (error is System.Management.ManagementException
            or UnauthorizedAccessException or System.Runtime.InteropServices.COMException)
        {
            // GPU details unavailable: the rest of the status is still reported.
        }
        foreach (var drive in DriveInfo.GetDrives())
        {
            if (status.Drives.Count >= MaxDrives) break;
            try
            {
                if (drive.DriveType != DriveType.Fixed || !drive.IsReady) continue;
                status.Drives.Add(new Protocol.DriveStatus
                {
                    Name = drive.Name.Length <= 8 ? drive.Name : drive.Name[..8],
                    TotalBytes = (ulong)drive.TotalSize,
                    FreeBytes = (ulong)drive.AvailableFreeSpace,
                });
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                // A drive that cannot be read is left out.
            }
        }
        return status;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct MemoryStatusEx
    {
        public uint Length;
        public uint MemoryLoad;
        public ulong TotalPhys;
        public ulong AvailPhys;
        public ulong TotalPageFile;
        public ulong AvailPageFile;
        public ulong TotalVirtual;
        public ulong AvailVirtual;
        public ulong AvailExtendedVirtual;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GlobalMemoryStatusEx(ref MemoryStatusEx buffer);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GetSystemTimes(out ulong idleTime, out ulong kernelTime, out ulong userTime);
}
