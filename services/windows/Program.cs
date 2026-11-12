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
        && command.CaptureWindow is null && command.ListProcesses is null && command.GetSystemStatus is null
        && command.ReadClipboard is null && command.WriteClipboard is null
        && command.ListWindows is null && command.ArrangeWindow is null && command.ListServices is null
        && command.InvokeControl is null && command.RunDiagnostic is null && command.Dictate is null
        && command.ListDisplays is null && command.CaptureDisplay is null && command.InspectProcess is null
        && command.ListApplications is null && command.StopProcess is null
        && (command.LaunchApplication is null
            || (command.LaunchApplication.Application != Protocol.LaunchApplication.Types.Application.Notepad
                && AppLauncher.Target(command.LaunchApplication.Application) is null))))
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
else if (command.StopProcess is { } stop)
{
    switch (ProcessLister.Stop(stop.ProcessId, stop.ExpectedName))
    {
        case "closed":
        case "ended":
            result.Status = Protocol.Result.Types.Status.Success;
            break;
        case "still_running":
            Reject(Protocol.Result.Types.Status.Partial, "still_running", "The process was asked to close and is still running.");
            break;
        case "protected_process":
            Reject(Protocol.Result.Types.Status.Denied, "protected_process", "This process is never stopped.");
            break;
        case "process_changed":
            Reject(Protocol.Result.Types.Status.Denied, "process_changed", "The process ID now names another program.");
            break;
        case "access_denied":
            Reject(Protocol.Result.Types.Status.Denied, "access_denied", "Windows refused to stop this process.");
            break;
        default:
            Reject(Protocol.Result.Types.Status.Failed, "process_unavailable", "The process does not exist.");
            break;
    }
}
else if (command.ListApplications is not null)
{
    result.ApplicationList = AppLauncher.Status();
    result.Status = Protocol.Result.Types.Status.Success;
}
else if (command.InspectProcess is { } inspectProcess)
{
    var details = ProcessLister.Inspect(inspectProcess.ProcessId);
    if (details is null)
        Reject(Protocol.Result.Types.Status.Failed, "process_unavailable", "The process does not exist.");
    else
    {
        result.ProcessDetails = details;
        result.Status = Protocol.Result.Types.Status.Success;
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
else if (command.Dictate is { } dictate)
{
    try
    {
        result.Dictation = Dictation.Listen(dictate.MaxSeconds, command.DeadlineMs);
        result.Status = Protocol.Result.Types.Status.Success;
    }
    catch (Exception error) when (error is InvalidOperationException or ArgumentException
        or PlatformNotSupportedException or System.Runtime.InteropServices.COMException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "dictation_failed", "Speech recognition failed.");
    }
}
else if (command.RunDiagnostic is { } diagnostic)
{
    if (!Diagnostics.Known(diagnostic.Diagnostic))
        Reject(Protocol.Result.Types.Status.Denied, "unknown_diagnostic", "Unknown diagnostic.");
    else if (Diagnostics.Run(diagnostic.Diagnostic, command.DeadlineMs) is { } diagnosticOutput)
    {
        result.CommandOutput = new Protocol.CommandOutput { Text = diagnosticOutput.Output, Truncated = diagnosticOutput.Truncated };
        result.Status = Protocol.Result.Types.Status.Success;
    }
    else
        Reject(Protocol.Result.Types.Status.Failed, "diagnostic_failed", "The diagnostic could not run.");
}
else if (command.InvokeControl is { } control)
{
    try
    {
        switch (WindowInspector.Invoke(control.Window, control.ProcessId, control.Index, control.ControlType,
                    control.Name, command.DeadlineMs, control.HasValue ? control.Value : null))
        {
            case true:
                result.Status = Protocol.Result.Types.Status.Success;
                break;
            case false:
                Reject(Protocol.Result.Types.Status.Failed, "not_invokable", "The control cannot be clicked.");
                break;
            default:
                Reject(Protocol.Result.Types.Status.Failed, "control_changed", "The window or control changed.");
                break;
        }
    }
    catch (Exception error) when (error is System.Windows.Automation.ElementNotAvailableException
        or System.Windows.Automation.ElementNotEnabledException or InvalidOperationException
        or System.Runtime.InteropServices.COMException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "control_changed", "The window or control changed.");
    }
}
else if (command.ListServices is { } listServices)
{
    if (listServices.Limit == 0 || listServices.Limit > ServiceLister.MaxLimit)
        Reject(Protocol.Result.Types.Status.Denied, "invalid_limit", "The limit must be between 1 and 25.");
    else
    {
        result.ServiceList = ServiceLister.Read(listServices.Limit);
        result.Status = Protocol.Result.Types.Status.Success;
    }
}
else if (command.ListWindows is { } listWindows)
{
    result.WindowList = WindowInspector.List(listWindows.ExcludeProcessId);
    result.Status = Protocol.Result.Types.Status.Success;
}
else if (command.ArrangeWindow is { } arrange)
{
    if (arrange.Arrangement == Protocol.ArrangeWindow.Types.Arrangement.Unspecified)
        Reject(Protocol.Result.Types.Status.Denied, "invalid_arrangement", "Unknown window arrangement.");
    else
        switch (WindowInspector.Arrange(arrange.Window, arrange.ProcessId, arrange.Arrangement))
        {
            case true:
                result.Status = Protocol.Result.Types.Status.Success;
                break;
            case false:
                Reject(Protocol.Result.Types.Status.Partial, "state_unverified", "The window state could not be verified.");
                break;
            default:
                Reject(Protocol.Result.Types.Status.Failed, "window_unavailable", "The window is no longer available.");
                break;
        }
}
else if (command.ReadClipboard is not null)
{
    var content = ClipboardText.Read(command.DeadlineMs);
    if (content is null)
        Reject(Protocol.Result.Types.Status.Failed, "clipboard_unavailable", "The clipboard could not be opened.");
    else
    {
        result.ClipboardContent = content;
        result.Status = Protocol.Result.Types.Status.Success;
    }
}
else if (command.WriteClipboard is { } writing)
{
    if (writing.Text.Length == 0 || writing.Text.Length > ClipboardText.MaxWrite * 2 || writing.Text.Contains('\0'))
        Reject(Protocol.Result.Types.Status.Denied, "invalid_text", "The text is empty, too long or invalid.");
    else
    {
        try
        {
            if (ClipboardText.Write(writing.Text, command.DeadlineMs))
                result.Status = Protocol.Result.Types.Status.Success;
            else
                Reject(Protocol.Result.Types.Status.Partial, "text_unverified", "The clipboard could not be read back.");
        }
        catch (Win32Exception)
        {
            Reject(Protocol.Result.Types.Status.Failed, "clipboard_unavailable", "The clipboard could not be written.");
        }
    }
}
else if (command.ListDisplays is not null)
{
    var displays = Displays.Read();
    if (displays is null)
        Reject(Protocol.Result.Types.Status.Failed, "displays_unavailable", "Display information is unavailable.");
    else
    {
        result.DisplayList = displays;
        result.Status = Protocol.Result.Types.Status.Success;
    }
}
else if (command.GetSystemStatus is not null)
{
    var status = SystemStatusReader.Read(command.DeadlineMs);
    if (status is null)
        Reject(Protocol.Result.Types.Status.Failed, "system_status_unavailable", "Windows could not report the system status.");
    else
    {
        result.SystemStatus = status;
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
else if (command.CaptureForeground is not null || command.CaptureDisplay is not null)
{
    try
    {
        var capture = command.CaptureDisplay is { } requested
            ? requested.Display is { } display ? WindowCapture.ReadDisplay(display, command.DeadlineMs) : null
            : WindowCapture.Read(command.DeadlineMs);
        if (capture is null)
            Reject(Protocol.Result.Types.Status.Failed, "capture_unavailable", "The requested capture is unavailable.");
        else
        {
            result.WindowCapture = capture;
            result.Status = Protocol.Result.Types.Status.Success;
        }
    }
    catch (Exception error) when (error is Win32Exception or IOException or ArgumentException
        or InvalidOperationException or System.Runtime.InteropServices.ExternalException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "capture_failed", "Windows could not capture the requested area.");
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
else if (command.LaunchApplication is { } other && AppLauncher.Target(other.Application) is { } app)
{
    try
    {
        if (AppLauncher.Launch(app.Path, app.ProcessName, command.DeadlineMs) is { } pid)
        {
            result.Status = Protocol.Result.Types.Status.Success;
            result.ProcessId = pid;
        }
        else
            Reject(Protocol.Result.Types.Status.Partial, "window_unverified", "Launch requested. No new window was verified.");
    }
    catch (Exception error) when (error is Win32Exception or InvalidOperationException or IOException)
    {
        Reject(Protocol.Result.Types.Status.Failed, "bridge_failure", "Windows could not start the application.");
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
