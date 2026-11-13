using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

internal static class Notepad
{
    private const string PackageFamily = "Microsoft.WindowsNotepad_8wekyb3d8bbwe";
    private static string SystemNotepad => Path.Combine(Environment.SystemDirectory, "notepad.exe");

    /// <summary>
    /// Starts the system Notepad without handle inheritance. Process.Start with
    /// UseShellExecute=false passes every inheritable handle to the child, so a launched
    /// application would keep the bridge's and host's pipes open for its whole lifetime.
    /// </summary>
    internal static void Start() => StartExecutable(SystemNotepad);

    /// <summary>Starts a fixed executable, without arguments nor handle inheritance.</summary>
    internal static void StartExecutable(string executable)
    {
        var startup = new StartupInfo { cb = Marshal.SizeOf<StartupInfo>() };
        char[] commandLine = $"\"{executable}\"\0".ToCharArray();
        if (!CreateProcessW(executable, commandLine, 0, 0, false, 0, 0,
                Environment.SystemDirectory, ref startup, out var created))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        CloseHandle(created.hThread);
        CloseHandle(created.hProcess);
    }

    internal static HashSet<(int Pid, nint Window)> Windows()
    {
        var windows = new HashSet<(int, nint)>();
        foreach (var process in Process.GetProcessesByName("Notepad"))
        {
            using (process)
            {
                try
                {
                    var window = process.MainWindowHandle;
                    if (window != 0 && IsNotepad(process))
                        windows.Add((process.Id, window));
                }
                catch (Exception error) when (error is Win32Exception or InvalidOperationException)
                {
                    // The process may have exited or be inaccessible.
                }
            }
        }
        return windows;
    }

    internal static bool IsNotepad(Process process)
    {
        uint length = 128;
        char[] family = new char[length];
        if (GetPackageFamilyName(process.SafeHandle, ref length, family) == 0)
            return string.Equals(new string(family, 0, (int)length).TrimEnd('\0'), PackageFamily, StringComparison.Ordinal);

        return string.Equals(process.MainModule?.FileName, SystemNotepad, StringComparison.OrdinalIgnoreCase);
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetPackageFamilyName(
        Microsoft.Win32.SafeHandles.SafeProcessHandle process,
        ref uint length,
        [Out] char[] family);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public int cb;
        public nint lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public nint lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation
    {
        public nint hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateProcessW(
        string applicationName, [In, Out] char[] commandLine, nint processAttributes, nint threadAttributes,
        [MarshalAs(UnmanagedType.Bool)] bool inheritHandles, uint creationFlags, nint environment,
        string currentDirectory, ref StartupInfo startupInfo, out ProcessInformation processInformation);

    [DllImport("kernel32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(nint handle);
}
