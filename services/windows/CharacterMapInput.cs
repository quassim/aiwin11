using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Automation;
using Protocol = Aiwin11.V1;

internal static class CharacterMapInput
{
    private const uint SetText = 0x000C;
    private const uint GetText = 0x000D;
    private const uint AbortIfHung = 0x0002;

    internal static void Apply(uint digit, long deadline, Protocol.Result result)
    {
        bool sent = false;
        try
        {
            nint window = GetForegroundWindow();
            if (window == 0 || GetWindowThreadProcessId(window, out int owner) == 0)
            {
                Reject();
                return;
            }
            using var process = Process.GetProcessById(owner);
            string executable = Path.Combine(Environment.SystemDirectory, "charmap.exe");
            if (!string.Equals(process.MainModule?.FileName, executable, StringComparison.OrdinalIgnoreCase))
            {
                Reject();
                return;
            }

            var root = AutomationElement.FromHandle(window);
            var field = root.FindFirst(TreeScope.Descendants, new AndCondition(
                new PropertyCondition(AutomationElement.AutomationIdProperty, "104"),
                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Edit)));
            if (field is null || field.Current.ProcessId != owner || !field.Current.IsEnabled
                || field.Current.IsOffscreen
                || !field.TryGetCurrentPattern(ValuePattern.Pattern, out object pattern)
                || pattern is not ValuePattern value || value.Current.IsReadOnly)
            {
                Reject();
                return;
            }
            nint fieldWindow = field.Current.NativeWindowHandle;
            if (fieldWindow == 0 || GetWindowThreadProcessId(fieldWindow, out int fieldOwner) == 0
                || fieldOwner != owner)
            {
                Reject();
                return;
            }
            if (GetForegroundWindow() != window
                || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline)
            {
                Reject();
                return;
            }

            string expected = digit.ToString(CultureInfo.InvariantCulture);
            sent = true;
            char[] payload = (expected + "\0").ToCharArray();
            if (SendMessageTimeoutW(fieldWindow, SetText, 0, payload, AbortIfHung, 250, out nuint written) == 0
                || written == 0)
            {
                Unverified();
                return;
            }
            var verification = Stopwatch.StartNew();
            while (verification.ElapsedMilliseconds < 1000
                   && DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() < deadline
                   && GetForegroundWindow() == window)
            {
                char[] actual = new char[8];
                if (SendMessageTimeoutW(fieldWindow, GetText, (nuint)actual.Length, actual, AbortIfHung, 100, out nuint copied) != 0
                    && copied == 1 && new string(actual, 0, 1) == expected)
                {
                    result.Status = Protocol.Result.Types.Status.Success;
                    return;
                }
                Thread.Sleep(30);
            }
            Unverified();
        }
        catch (Exception error) when (error is ElementNotAvailableException or ElementNotEnabledException
            or COMException or Win32Exception or ArgumentException or InvalidOperationException
            or UnauthorizedAccessException)
        {
            if (sent) Unverified();
            else Reject();
        }

        void Reject()
        {
            result.Status = Protocol.Result.Types.Status.Denied;
            result.Error = new Protocol.Error { Code = "character_map_unavailable", Message = "The Character Map target could not be verified." };
        }

        void Unverified()
        {
            result.Status = Protocol.Result.Types.Status.Partial;
            result.Error = new Protocol.Error { Code = "field_unverified", Message = "The Character Map field could not be verified." };
        }
    }

    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out int pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern nint SendMessageTimeoutW(
        nint window, uint message, nuint capacity, [In, Out] char[] text,
        uint flags, uint timeout, out nuint result);
}
