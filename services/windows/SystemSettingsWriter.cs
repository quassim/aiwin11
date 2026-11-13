using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32;
using Protocol = Aiwin11.V1;

/// Updates a closed set of standard-user Windows personalization settings in HKCU
/// and notifies top-level windows of the change.
internal static class SystemSettingsWriter
{
    const string PersonalizeKey = @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    static readonly IntPtr HwndBroadcast = new(0xffff);
    const uint WmSettingChange = 0x001A;
    const uint SmtoAbortIfHung = 0x0002;

    public static string Apply(Protocol.SetSystemSetting.Types.Setting setting, long deadlineMs)
    {
        if (DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadlineMs) return "expired";
        try
        {
            using var key = Registry.CurrentUser.CreateSubKey(PersonalizeKey, writable: true);
            if (key is null) return "failed";
            switch (setting)
            {
                case Protocol.SetSystemSetting.Types.Setting.ThemeLight:
                    key.SetValue("AppsUseLightTheme", 1, RegistryValueKind.DWord);
                    key.SetValue("SystemUsesLightTheme", 1, RegistryValueKind.DWord);
                    break;
                case Protocol.SetSystemSetting.Types.Setting.ThemeDark:
                    key.SetValue("AppsUseLightTheme", 0, RegistryValueKind.DWord);
                    key.SetValue("SystemUsesLightTheme", 0, RegistryValueKind.DWord);
                    break;
                case Protocol.SetSystemSetting.Types.Setting.TransparencyOn:
                    key.SetValue("EnableTransparency", 1, RegistryValueKind.DWord);
                    break;
                case Protocol.SetSystemSetting.Types.Setting.TransparencyOff:
                    key.SetValue("EnableTransparency", 0, RegistryValueKind.DWord);
                    break;
                default:
                    return "invalid_setting";
            }
            SendMessageTimeoutW(HwndBroadcast, WmSettingChange, UIntPtr.Zero, "ImmersiveColorSet", SmtoAbortIfHung, 500, out _);
            return "ok";
        }
        catch (Exception error) when (error is UnauthorizedAccessException or IOException or System.Security.SecurityException)
        {
            return "failed";
        }
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr SendMessageTimeoutW(
        IntPtr hWnd, uint msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
}
