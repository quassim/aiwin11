using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;
using Protocol = Aiwin11.V1;

/// ShellService.ExecuteApprovedCommand: a closed list of read-only Windows diagnostics, each a
/// fixed System32 executable with fixed arguments. PowerShell runs only a compiled-in query.
static class Diagnostics
{
    const int MaxOutput = 12_000;

    static (string Executable, string Arguments)? Command(Protocol.RunDiagnostic.Types.Diagnostic diagnostic) =>
        diagnostic switch
        {
            Protocol.RunDiagnostic.Types.Diagnostic.Network => ("ipconfig.exe", "/all"),
            Protocol.RunDiagnostic.Types.Diagnostic.Wifi => ("netsh.exe", "wlan show interfaces"),
            Protocol.RunDiagnostic.Types.Diagnostic.Time => ("w32tm.exe", "/query /status"),
            Protocol.RunDiagnostic.Types.Diagnostic.PowerShellOs =>
                (@"WindowsPowerShell\v1.0\powershell.exe",
                 "-NoLogo -NoProfile -NonInteractive -Command \"$ErrorActionPreference = 'Stop'; " +
                 "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; " +
                 "Get-CimInstance -ClassName Win32_OperatingSystem | " +
                 "Select-Object Caption,Version,BuildNumber | ConvertTo-Json -Compress\""),
            _ => null,
        };

    public static bool Known(Protocol.RunDiagnostic.Types.Diagnostic diagnostic) => Command(diagnostic) is not null;

    /// Output (OEM code page decoded), truncated to 12 000 characters; null on failure.
    public static (string Output, bool Truncated)? Run(Protocol.RunDiagnostic.Types.Diagnostic diagnostic, long deadlineMs)
    {
        if (Command(diagnostic) is not { } command) return null;
        Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
        var start = new ProcessStartInfo(Path.Combine(Environment.SystemDirectory, command.Executable), command.Arguments)
        {
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
            WorkingDirectory = Environment.SystemDirectory,
            StandardOutputEncoding = diagnostic == Protocol.RunDiagnostic.Types.Diagnostic.PowerShellOs
                ? Encoding.UTF8 : Encoding.GetEncoding(CultureInfo.CurrentCulture.TextInfo.OEMCodePage),
        };
        try
        {
            using var process = Process.Start(start);
            if (process is null) return null;
            var output = process.StandardOutput.ReadToEndAsync();
            _ = process.StandardError.ReadToEndAsync();
            var wait = (int)Math.Clamp(deadlineMs - DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), 0, 10_000);
            if (!process.WaitForExit(wait))
            {
                process.Kill();
                return null;
            }
            var text = new string(output.Result.Where(c => c == '\n' || !char.IsControl(c)).ToArray()).Trim();
            if (diagnostic == Protocol.RunDiagnostic.Types.Diagnostic.PowerShellOs && (process.ExitCode != 0 || text.Length == 0))
                return null;
            return text.Length <= MaxOutput ? (text, false) : (text[..MaxOutput], true);
        }
        catch (Exception error) when (error is Win32Exception or InvalidOperationException or IOException)
        {
            return null;
        }
    }
}
