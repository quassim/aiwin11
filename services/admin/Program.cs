using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.ServiceProcess;
using System.Text.RegularExpressions;

// AiWin11 elevated helper (ROADMAP §6, option B). Started by the core through UAC ("runas")
// only after the policy engine and the user approved one operation. It performs exactly one
// operation from a closed list, then exits. It re-validates everything itself: it trusts no
// caller, reads no file, opens no listener and never runs a shell.
//
// Usage:  AiWin11.Admin.exe service <start|stop|restart|automatic|manual|disabled> <name>
//         AiWin11.Admin.exe clean-temp
//         AiWin11.Admin.exe restore-point <label>
// Exit codes: 0 done, 2 invalid request, 3 refused (protected), 4 failed, 5 not elevated.

const int Invalid = 2, Failed = 4, NotElevated = 5;

if (!OperatingSystem.IsWindows()) return Failed;
using (var identity = WindowsIdentity.GetCurrent())
{
    if (!new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator)) return NotElevated;
}

try
{
    return args switch
    {
        ["service", var action, var name] => Services.Run(action, name),
        ["clean-temp"] => Temp.Clean(),
        ["restore-point", var label] => RestorePoints.Create(label),
        _ => Invalid,
    };
}
catch (Exception error) when (error is InvalidOperationException or Win32Exception
    or System.ServiceProcess.TimeoutException or UnauthorizedAccessException or IOException)
{
    return Failed;
}

static class Services
{
    // Never touched, whatever the request: security, updates and services Windows needs to run.
    public static readonly HashSet<string> Protected = new(StringComparer.OrdinalIgnoreCase)
    {
        "WinDefend", "WdNisSvc", "WdFilter", "Sense", "SecurityHealthService", "wscsvc", "MpsSvc", "BFE",
        "mpssvc", "wuauserv", "UsoSvc", "WaaSMedicSvc", "TrustedInstaller", "RpcSs", "RpcEptMapper",
        "DcomLaunch", "LSM", "SamSs", "EventLog", "Winmgmt", "CryptSvc", "Power", "PlugPlay", "ProfSvc",
        "Schedule", "SystemEventsBroker", "BrokerInfrastructure", "CoreMessagingRegistrar", "gpsvc",
        "Dhcp", "Dnscache", "nsi", "Netlogon", "KeyIso", "VaultSvc", "AppIDSvc", "EFS", "BITS",
    };

    static readonly Regex ValidName = new("^[A-Za-z0-9_.-]{1,64}$", RegexOptions.CultureInvariant);
    static readonly TimeSpan Wait = TimeSpan.FromSeconds(30);

    public static int Run(string action, string name)
    {
        if (!ValidName.IsMatch(name)) return 2;
        if (Protected.Contains(name)) return 3;
        using var service = new ServiceController(name);
        _ = service.Status; // Throws InvalidOperationException for an unknown service.
        switch (action)
        {
            case "start":
                if (service.Status != ServiceControllerStatus.Running) service.Start();
                service.WaitForStatus(ServiceControllerStatus.Running, Wait);
                return 0;
            case "stop":
                if (service.Status != ServiceControllerStatus.Stopped) service.Stop();
                service.WaitForStatus(ServiceControllerStatus.Stopped, Wait);
                return 0;
            case "restart":
                if (service.Status != ServiceControllerStatus.Stopped)
                {
                    service.Stop();
                    service.WaitForStatus(ServiceControllerStatus.Stopped, Wait);
                }
                service.Start();
                service.WaitForStatus(ServiceControllerStatus.Running, Wait);
                return 0;
            case "automatic": return SetStart(name, 2);
            case "manual": return SetStart(name, 3);
            case "disabled": return SetStart(name, 4);
            default: return 2;
        }
    }

    static int SetStart(string name, uint startType)
    {
        const uint ScManagerConnect = 0x0001, ServiceChangeConfig = 0x0002, NoChange = 0xFFFFFFFF;
        nint manager = OpenSCManagerW(null, null, ScManagerConnect);
        if (manager == 0) return 4;
        try
        {
            nint service = OpenServiceW(manager, name, ServiceChangeConfig);
            if (service == 0) return 4;
            try
            {
                return ChangeServiceConfigW(service, NoChange, startType, NoChange, null, null, 0, null, null, null, null)
                    ? 0 : 4;
            }
            finally { CloseServiceHandle(service); }
        }
        finally { CloseServiceHandle(manager); }
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern nint OpenSCManagerW(string? machine, string? database, uint access);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern nint OpenServiceW(nint manager, string name, uint access);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool ChangeServiceConfigW(nint service, uint serviceType, uint startType, uint errorControl,
        string? binaryPath, string? loadOrderGroup, nint tagId, string? dependencies, string? startName,
        string? password, string? displayName);
    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CloseServiceHandle(nint handle);
}

static class Temp
{
    /// Deletes files older than one day directly in %SystemRoot%\Temp and its subfolders,
    /// never following links or junctions; files in use are skipped.
    public static int Clean()
    {
        var root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "Temp");
        if (!Directory.Exists(root)) return 0;
        var limit = DateTime.UtcNow.AddDays(-1);
        var options = new EnumerationOptions
        {
            RecurseSubdirectories = true,
            AttributesToSkip = FileAttributes.ReparsePoint,
            IgnoreInaccessible = true,
        };
        foreach (var path in Directory.EnumerateFiles(root, "*", options))
        {
            try
            {
                var info = new FileInfo(path);
                if ((info.Attributes & FileAttributes.ReparsePoint) != 0 || info.LastWriteTimeUtc > limit) continue;
                info.Delete();
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                // In use or protected: left in place.
            }
        }
        return 0;
    }
}

static class RestorePoints
{
    const uint BeginSystemChange = 100;
    const uint EndSystemChange = 101;
    const uint ModifySettings = 12;

    public static int Create(string rawLabel)
    {
        var label = rawLabel.Trim();
        if (label.Length is < 1 or > 64 || label.Any(char.IsControl)) return 2;
        var begin = new RestorePointInfo
        {
            EventType = BeginSystemChange,
            RestorePointType = ModifySettings,
            SequenceNumber = 0,
            Description = label,
        };
        if (!SRSetRestorePointW(ref begin, out var status)) return 4;
        var end = new RestorePointInfo
        {
            EventType = EndSystemChange,
            RestorePointType = ModifySettings,
            SequenceNumber = status.SequenceNumber,
            Description = label,
        };
        _ = SRSetRestorePointW(ref end, out _);
        return 0;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct RestorePointInfo
    {
        public uint EventType;
        public uint RestorePointType;
        public long SequenceNumber;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string Description;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct StateMgrStatus
    {
        public uint Status;
        public long SequenceNumber;
    }

    [DllImport("srclient.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SRSetRestorePointW(ref RestorePointInfo pRestorePtSpec, out StateMgrStatus pSMgrStatus);
}
