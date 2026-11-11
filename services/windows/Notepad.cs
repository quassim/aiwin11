using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

internal static class Notepad
{
    private const string PackageFamily = "Microsoft.WindowsNotepad_8wekyb3d8bbwe";
    private static string SystemNotepad => Path.Combine(Environment.SystemDirectory, "notepad.exe");

    internal static Process Start() =>
        Process.Start(new ProcessStartInfo(SystemNotepad)
        {
            WorkingDirectory = Environment.SystemDirectory,
            UseShellExecute = false,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        }) ?? throw new InvalidOperationException("Notepad did not start.");

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

    private static bool IsNotepad(Process process)
    {
        uint length = 128;
        var family = new StringBuilder((int)length);
        if (GetPackageFamilyName(process.SafeHandle, ref length, family) == 0)
            return string.Equals(family.ToString(), PackageFamily, StringComparison.Ordinal);

        return string.Equals(process.MainModule?.FileName, SystemNotepad, StringComparison.OrdinalIgnoreCase);
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetPackageFamilyName(
        Microsoft.Win32.SafeHandles.SafeProcessHandle process,
        ref uint length,
        StringBuilder family);

}
