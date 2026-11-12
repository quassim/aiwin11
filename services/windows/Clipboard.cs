using System.ComponentModel;
using System.Runtime.InteropServices;
using Protocol = Aiwin11.V1;

/// Clipboard text for the agent's `read_clipboard` and `write_clipboard` tools, through the
/// Win32 clipboard API (Unicode text only). Both run only after the user approved them.
static class ClipboardText
{
    public const int MaxRead = 4000;
    public const int MaxWrite = 2000;
    const uint UnicodeText = 13;
    const uint MoveableMemory = 0x0002;

    /// Reads the clipboard's text, at most `MaxRead` characters. Null if it cannot be opened.
    public static Protocol.ClipboardContent? Read(long deadlineMs)
    {
        if (!Open(deadlineMs)) return null;
        try
        {
            var content = new Protocol.ClipboardContent();
            if (!IsClipboardFormatAvailable(UnicodeText)) return content;
            var handle = GetClipboardData(UnicodeText);
            if (handle == 0) return content;
            var pointer = GlobalLock(handle);
            if (pointer == 0) return null;
            try
            {
                var text = Marshal.PtrToStringUni(pointer) ?? "";
                text = text.Replace("\0", "");
                content.Truncated = text.Length > MaxRead;
                content.Text = content.Truncated ? text[..MaxRead] : text;
                content.HasText = text.Length > 0;
                return content;
            }
            finally
            {
                GlobalUnlock(handle);
            }
        }
        finally
        {
            CloseClipboard();
        }
    }

    /// Replaces the clipboard with `text`, then reads it back. Returns whether it matches.
    public static bool Write(string text, long deadlineMs)
    {
        if (!Open(deadlineMs)) throw new Win32Exception("clipboard_unavailable");
        try
        {
            if (!EmptyClipboard()) throw new Win32Exception(Marshal.GetLastWin32Error());
            var bytes = (text.Length + 1) * 2;
            var memory = GlobalAlloc(MoveableMemory, (nuint)bytes);
            if (memory == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            var pointer = GlobalLock(memory);
            if (pointer == 0)
            {
                GlobalFree(memory);
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            try
            {
                Marshal.Copy(text.ToCharArray(), 0, pointer, text.Length);
                Marshal.WriteInt16(pointer, text.Length * 2, 0);
            }
            finally
            {
                GlobalUnlock(memory);
            }
            if (SetClipboardData(UnicodeText, memory) == 0)
            {
                GlobalFree(memory);
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            // The system owns the memory from now on.
        }
        finally
        {
            CloseClipboard();
        }
        var check = Read(deadlineMs);
        return check is not null && check.Text == text;
    }

    /// Another application may hold the clipboard briefly: retry until the deadline.
    static bool Open(long deadlineMs)
    {
        while (DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() < deadlineMs)
        {
            if (OpenClipboard(0)) return true;
            Thread.Sleep(50);
        }
        return false;
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool OpenClipboard(nint owner);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CloseClipboard();

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool EmptyClipboard();

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool IsClipboardFormatAvailable(uint format);

    [DllImport("user32.dll", SetLastError = true)]
    static extern nint GetClipboardData(uint format);

    [DllImport("user32.dll", SetLastError = true)]
    static extern nint SetClipboardData(uint format, nint memory);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern nint GlobalAlloc(uint flags, nuint bytes);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern nint GlobalFree(nint memory);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern nint GlobalLock(nint memory);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GlobalUnlock(nint memory);
}
