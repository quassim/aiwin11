using System.ComponentModel;
using System.Diagnostics;

/// Opens one strictly validated https:// URL in the user's default Windows browser.
internal static class BrowserLauncher
{
    internal const int MaxUrlLength = 512;

    internal static bool IsAllowedHttpsUrl(string? raw, out Uri? parsed)
    {
        parsed = null;
        if (string.IsNullOrWhiteSpace(raw) || raw.Length > MaxUrlLength) return false;
        if (raw.Any(ch => char.IsWhiteSpace(ch) || char.IsControl(ch))) return false;
        if (!Uri.TryCreate(raw, UriKind.Absolute, out var uri)) return false;
        if (!string.Equals(uri.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase)) return false;
        if (!string.IsNullOrEmpty(uri.UserInfo) || string.IsNullOrEmpty(uri.Host)) return false;
        if (uri.IsLoopback) return false;
        var host = uri.Host.ToLowerInvariant();
        if (host == "localhost" || host.EndsWith(".local", StringComparison.Ordinal) || host.EndsWith(".internal", StringComparison.Ordinal))
            return false;
        parsed = uri;
        return true;
    }

    internal static bool Open(string raw)
    {
        if (!IsAllowedHttpsUrl(raw, out var uri) || uri is null) return false;
        try
        {
            using var process = Process.Start(new ProcessStartInfo(uri.AbsoluteUri)
            {
                UseShellExecute = true,
            });
            return true;
        }
        catch (Win32Exception)
        {
            return false;
        }
        catch (InvalidOperationException)
        {
            return false;
        }
    }
}
