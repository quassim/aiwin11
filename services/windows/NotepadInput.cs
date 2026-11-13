using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using Protocol = Aiwin11.V1;

/// <summary>
/// Writes text into the editor of one Notepad process, the one the same task launched and
/// verified. Never overwrites: the document must be empty. No keystrokes are simulated.
/// </summary>
internal static class NotepadInput
{
    internal const int MaxText = 2000;
    private const uint SetText = 0x000C;
    private const uint GetText = 0x000D;
    private const uint GetTextLength = 0x000E;
    private const uint AbortIfHung = 0x0002;
    // Windows 11 Notepad uses RichEditD2DPT; the classic one uses Edit.
    private static readonly string[] EditorClasses = { "RichEditD2DPT", "Edit" };

    internal static void Apply(uint processId, string text, long deadline, Protocol.Result result)
    {
        bool sent = false;
        try
        {
            using var process = Process.GetProcessById((int)processId);
            if (!Notepad.IsNotepad(process))
            {
                Reject("notepad_unavailable");
                return;
            }
            // The editor of a freshly opened window can appear shortly after the window.
            nint editor = 0;
            var wait = Stopwatch.StartNew();
            while (editor == 0 && wait.ElapsedMilliseconds < 2000 && Now() < deadline)
            {
                process.Refresh();
                editor = FindEditor(process.MainWindowHandle, processId);
                if (editor == 0) Thread.Sleep(100);
            }
            if (editor == 0)
            {
                Reject("notepad_unavailable");
                return;
            }
            // Never overwrite: restored tabs or user text make the target unsuitable.
            if (SendMessageTimeoutW(editor, GetTextLength, 0, 0, AbortIfHung, 250, out nuint length) == 0)
            {
                Reject("notepad_unavailable");
                return;
            }
            if (length != 0)
            {
                Reject("document_not_empty");
                return;
            }
            if (Now() >= deadline)
            {
                Reject("expired");
                return;
            }

            string expected = text.Replace("\r\n", "\n").Replace('\r', '\n').Replace("\n", "\r\n");
            sent = true;
            char[] expectedChars = (expected + "\0").ToCharArray();
            if (SendMessageTimeoutW(editor, SetText, 0, expectedChars, AbortIfHung, 1000, out nuint accepted) == 0
                || accepted == 0)
            {
                Unverified();
                return;
            }
            char[] actual = new char[expected.Length * 2 + 16];
            if (SendMessageTimeoutW(editor, GetText, (nuint)actual.Length, actual, AbortIfHung, 1000, out nuint copied) != 0
                && Normalize(new string(actual, 0, Math.Min((int)copied, actual.Length)).TrimEnd('\0')) == Normalize(expected))
            {
                result.Status = Protocol.Result.Types.Status.Success;
                return;
            }
            Unverified();
        }
        catch (Exception error) when (error is ArgumentException or InvalidOperationException
            or Win32Exception or COMException or UnauthorizedAccessException)
        {
            if (sent) Unverified();
            else Reject("notepad_unavailable");
        }

        void Reject(string code)
        {
            result.Status = Protocol.Result.Types.Status.Denied;
            result.Error = new Protocol.Error { Code = code, Message = "Notepad text input was not performed." };
        }

        void Unverified()
        {
            result.Status = Protocol.Result.Types.Status.Partial;
            result.Error = new Protocol.Error { Code = "text_unverified", Message = "The text was sent but could not be verified." };
        }
    }

    private static long Now() => DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();

    // RichEdit may report paragraph ends as CR or CRLF.
    private static string Normalize(string text) => text.Replace("\r", "");

    private static nint FindEditor(nint topLevel, uint processId)
    {
        if (topLevel == 0)
            return 0;
        nint found = 0;
        EnumChildWindows(topLevel, (child, _) =>
        {
            char[] name = new char[32];
            int nameLength = GetClassNameW(child, name, name.Length);
            if (IsWindowVisible(child) && nameLength > 0
                && Array.IndexOf(EditorClasses, new string(name, 0, nameLength)) >= 0
                && GetWindowThreadProcessId(child, out uint owner) != 0 && owner == processId)
            {
                found = child;
                return false;
            }
            return true;
        }, 0);
        return found;
    }

    private delegate bool EnumProc(nint window, nint parameter);

    [DllImport("user32.dll")] private static extern bool EnumChildWindows(nint parent, EnumProc callback, nint parameter);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetClassNameW(nint window, [Out] char[] className, int maxCount);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern nint SendMessageTimeoutW(
        nint window, uint message, nuint wParam, [In, Out] char[] lParam, uint flags, uint timeout, out nuint result);
    [DllImport("user32.dll", ExactSpelling = true)]
    private static extern nint SendMessageTimeoutW(
        nint window, uint message, nuint wParam, nint lParam, uint flags, uint timeout, out nuint result);
}
