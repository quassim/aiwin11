using System.Management;
using Protocol = Aiwin11.V1;

/// Read-only snapshot of connected user-facing Windows peripherals (audio, printers, cameras,
/// monitors, Bluetooth and USB/HID devices) for the agent's `list_peripherals` tool.
internal static class PeripheralsReader
{
    const int MaxPeripherals = 40;
    const int MaxNameChars = 120;

    static readonly Dictionary<string, string> CategoryLabels = new(StringComparer.OrdinalIgnoreCase)
    {
        ["AudioEndpoint"] = "audio",
        ["MEDIA"] = "audio",
        ["Printer"] = "printer",
        ["PrintQueue"] = "printer",
        ["Camera"] = "camera",
        ["Image"] = "camera",
        ["Monitor"] = "monitor",
        ["Bluetooth"] = "bluetooth",
        ["HIDClass"] = "input",
        ["Keyboard"] = "input",
        ["Mouse"] = "input",
    };

    internal static Protocol.PeripheralList Read()
    {
        var list = new Protocol.PeripheralList();
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        try
        {
            using var searcher = new ManagementObjectSearcher(
                "SELECT Name, PNPClass, Status FROM Win32_PnPEntity WHERE Present = True");
            foreach (var device in searcher.Get())
            {
                using (device)
                {
                    if (list.Peripherals.Count >= MaxPeripherals) break;
                    var pnpClass = (device["PNPClass"] as string ?? "").Trim();
                    if (!CategoryLabels.TryGetValue(pnpClass, out var category)) continue;
                    var name = (device["Name"] as string ?? "").Trim();
                    if (name.Length == 0 || IsGenericVirtualDevice(name)) continue;
                    var trimmedName = name.Length <= MaxNameChars ? name : name[..MaxNameChars];
                    var key = $"{category}:{trimmedName}";
                    if (!seen.Add(key)) continue;
                    var status = (device["Status"] as string ?? "OK").Trim();
                    list.Peripherals.Add(new Protocol.PeripheralInfo
                    {
                        Name = trimmedName,
                        Category = category,
                        Status = status.Length <= 32 ? status : status[..32],
                    });
                }
            }
        }
        catch (ManagementException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
        return list;
    }

    static bool IsGenericVirtualDevice(string name) =>
        name.StartsWith("HID-compliant", StringComparison.OrdinalIgnoreCase)
        || name.StartsWith("Périphérique ", StringComparison.OrdinalIgnoreCase)
        || name.StartsWith("Generic ", StringComparison.OrdinalIgnoreCase)
        || name.Equals("Microsoft Print to PDF", StringComparison.OrdinalIgnoreCase);
}
