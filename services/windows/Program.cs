using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using Google.Protobuf;
using Protocol = Aiwin11.V1;

// Large enough for 2000 characters of Notepad text (UTF-8) plus the envelope.
const int maxMessageBytes = 16 * 1024;
using var input = Console.OpenStandardInput();
using var output = Console.OpenStandardOutput();
var bytes = new byte[maxMessageBytes + 1];
int length = 0;
while (length < bytes.Length)
{
    int count = input.Read(bytes, length, bytes.Length - length);
    if (count == 0) break;
    length += count;
}
// A spare bridge whose host exited sees an empty stdin: leave without writing anything.
if (length == 0) return 3;
if (length > maxMessageBytes) return 2;

Protocol.Command command;
try
{
    command = Protocol.Command.Parser.ParseFrom(bytes, 0, length);
}
catch (InvalidProtocolBufferException)
{
    return 2;
}

var result = new Protocol.Result { SchemaVersion = 1, RequestId = command.RequestId };
var elapsed = Stopwatch.StartNew();
if (command.SchemaVersion != 1 || !Guid.TryParseExact(command.RequestId, "D", out _)
    || (command.InspectForeground is null && command.CaptureForeground is null && command.CharacterMapDigit is null
        && command.LocateWindow is null && command.InspectWindow is null && command.TypeInNotepad is null
        && command.CaptureWindow is null && command.ListProcesses is null
        && command.LaunchApplication?.Application != Protocol.LaunchApplication.Types.Application.Notepad))
{
    Reject(Protocol.Result.Types.Status.Denied, "unsupported_command", "Unsupported bridge command.");
}
else if (command.DeadlineMs <= DateTimeOffset.UtcNow.ToUnixTimeMilliseconds())
{
    Reject(Protocol.Result.Types.Status.Timeout, "expired", "The command deadline expired.");
}
else if (!OperatingSystem.IsWindows() || !Environment.UserInteractive)
{
    Reject(Protocol.Result.Types.Status.Failed, "interactive_windows_required", "An interactive Windows session is required.");
}
else if (command.CharacterMapDigit is { } entry)
{
    if (!entry.HasDigit || entry.Digit > 9)
        Reject(Protocol.Result.Types.Status.Denied, "invalid_digit", "A decimal digit is required.");
    else
        CharacterMapInput.Apply(entry.Digit, command.DeadlineMs, result);
}
else if (command.TypeInNotepad is { } typing)
{
    if (typing.Text.Length == 0 || typing.Text.Length > NotepadInput.MaxText * 2 || typing.Text.Contains('\0'))
        Reject(Protocol.Result.Types.Status.Denied, "invalid_text", "The text is empty, too long or invalid.");
    else
        NotepadInput.Apply(typing.ProcessId, typing.Text, command.DeadlineMs, result);
}
else if (command.CaptureWindow is { } captureTarget)
{
    try
    {
        var frame = WindowCapture.ReadWindow(captureTarget.Window, captureTarget.ProcessId, command.DeadlineMs);
        if (frame is null)
            Reject(Protocol.Result.Types.Status.Failed, "capture_unavailable", "The window could not be captured.");
        else
        {
            result.WindowCapture = frame;
            result.Status = Protocol.Result.Types.Status.Success;
        }
    }
    catch (Exception error) when (error is Win32Exception or IOException or ArgumentException
        or InvalidOperationException or System.Runtime.InteropServices.ExternalException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "capture_failed", "Windows could not capture the window.");
    }
}
else if (command.ListProcesses is { } listing)
{
    if (listing.Limit == 0 || listing.Limit > ProcessLister.MaxLimit)
        Reject(Protocol.Result.Types.Status.Denied, "invalid_limit", "The limit must be between 1 and 25.");
    else
    {
        result.ProcessList = ProcessLister.Read(listing.Limit);
        result.Status = Protocol.Result.Types.Status.Success;
    }
}
else if (command.LocateWindow is { } locate)
{
    var info = WindowInspector.Locate(locate.ExcludeProcessId);
    if (info is null)
        Reject(Protocol.Result.Types.Status.Failed, "window_unavailable", "No application window was found.");
    else
    {
        result.WindowInfo = info;
        result.Status = Protocol.Result.Types.Status.Success;
    }
}
else if (command.InspectWindow is { } target)
{
    try
    {
        var tree = WindowInspector.Inspect(target.Window, target.ProcessId, command.DeadlineMs);
        if (tree is null)
            Reject(Protocol.Result.Types.Status.Failed, "window_unavailable", "The window is no longer available.");
        else
        {
            result.UiTree = tree;
            result.Status = Protocol.Result.Types.Status.Success;
        }
    }
    catch (Exception error) when (error is System.Windows.Automation.ElementNotAvailableException
        or System.Runtime.InteropServices.COMException or UnauthorizedAccessException
        or ArgumentException or InvalidOperationException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "inspection_failed", "Windows could not inspect the window.");
    }
}
else if (command.CaptureForeground is not null)
{
    try
    {
        var capture = WindowCapture.Read(command.DeadlineMs);
        if (capture is null)
            Reject(Protocol.Result.Types.Status.Failed, "capture_unavailable", "The foreground window could not be captured.");
        else
        {
            result.WindowCapture = capture;
            result.Status = Protocol.Result.Types.Status.Success;
        }
    }
    catch (Exception error) when (error is Win32Exception or IOException or ArgumentException
        or InvalidOperationException or System.Runtime.InteropServices.ExternalException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "capture_failed", "Windows could not capture the foreground window.");
    }
}
else if (command.InspectForeground is not null)
{
    try
    {
        var tree = UiInspector.Read(command.DeadlineMs);
        if (tree is null)
            Reject(Protocol.Result.Types.Status.Failed, "window_unavailable", "The foreground window could not be inspected.");
        else
        {
            result.UiTree = tree;
            result.Status = Protocol.Result.Types.Status.Success;
        }
    }
    catch (Exception error) when (error is System.Windows.Automation.ElementNotAvailableException
        or System.Runtime.InteropServices.COMException or UnauthorizedAccessException
        or ArgumentException or InvalidOperationException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "inspection_failed", "Windows could not inspect the foreground window.");
    }
}
// Explicit: a command without a dedicated branch above must never fall into the launch.
else if (command.LaunchApplication?.Application == Protocol.LaunchApplication.Types.Application.Notepad)
{
    try
    {
        var existing = Notepad.Windows();
        if (command.DeadlineMs <= DateTimeOffset.UtcNow.ToUnixTimeMilliseconds())
        {
            Reject(Protocol.Result.Types.Status.Timeout, "expired", "The command deadline expired.");
        }
        else
        {
            Notepad.Start();
            var verification = Stopwatch.StartNew();
            while (verification.Elapsed < TimeSpan.FromSeconds(4)
                   && DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() < command.DeadlineMs)
            {
                var opened = Notepad.Windows().Except(existing).FirstOrDefault();
                if (opened != default)
                {
                    result.Status = Protocol.Result.Types.Status.Success;
                    result.ProcessId = (uint)opened.Pid;
                    break;
                }
                Thread.Sleep(75);
            }
            if (!result.HasProcessId)
                Reject(Protocol.Result.Types.Status.Partial, "window_unverified", "Launch requested. No new Notepad window was verified.");
        }
    }
    catch (Exception error) when (error is Win32Exception or InvalidOperationException or IOException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "bridge_failure", "Windows could not complete the Notepad operation.");
    }
}
else
{
    Reject(Protocol.Result.Types.Status.Denied, "unsupported_command", "Unsupported bridge command.");
}

result.DurationMs = (ulong)elapsed.ElapsedMilliseconds;
result.WriteTo(output);
return 0;

void Reject(Protocol.Result.Types.Status status, string code, string message)
{
    result.Status = status;
    result.Error = new Protocol.Error { Code = code, Message = message };
}
