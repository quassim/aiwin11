using System.ComponentModel;
using System.ServiceProcess;
using Protocol = Aiwin11.V1;

/// Read-only Windows services snapshot for the agent's `list_services` tool: automatic
/// services that are stopped first (often the sign of a problem), then running services.
/// Never starts, stops or reconfigures anything.
static class ServiceLister
{
    public const uint MaxLimit = 25;
    const int MaxName = 64;
    const int MaxDisplayName = 120;

    public static Protocol.ServiceList Read(uint limit)
    {
        var list = new Protocol.ServiceList();
        var entries = new List<(int Rank, Protocol.ServiceEntry Entry)>();
        foreach (var service in ServiceController.GetServices())
        {
            using (service)
            {
                try
                {
                    list.Total++;
                    var status = service.Status;
                    var start = service.StartType;
                    var entry = new Protocol.ServiceEntry
                    {
                        Name = Clean(service.ServiceName, MaxName),
                        DisplayName = Clean(service.DisplayName, MaxDisplayName),
                        Running = status == ServiceControllerStatus.Running,
                        StartMode = start switch
                        {
                            ServiceStartMode.Automatic => Protocol.ServiceEntry.Types.StartMode.Automatic,
                            ServiceStartMode.Manual => Protocol.ServiceEntry.Types.StartMode.Manual,
                            ServiceStartMode.Disabled => Protocol.ServiceEntry.Types.StartMode.Disabled,
                            _ => Protocol.ServiceEntry.Types.StartMode.Other,
                        },
                    };
                    if (entry.Name.Length == 0) continue;
                    if (entry.Running) list.Running++;
                    bool stoppedAutomatic = !entry.Running && start == ServiceStartMode.Automatic;
                    if (stoppedAutomatic) list.StoppedAutomatic++;
                    if (stoppedAutomatic || entry.Running)
                        entries.Add((stoppedAutomatic ? 0 : 1, entry));
                }
                catch (Exception error) when (error is InvalidOperationException or Win32Exception)
                {
                    // A service that cannot be queried by a standard user is left out.
                }
            }
        }
        list.Services.AddRange(entries.OrderBy(item => item.Rank)
            .ThenBy(item => item.Entry.Name, StringComparer.OrdinalIgnoreCase)
            .Take((int)Math.Min(limit, MaxLimit)).Select(item => item.Entry));
        return list;
    }

    static string Clean(string? value, int max)
    {
        if (string.IsNullOrEmpty(value)) return "";
        var text = new string(value.Select(character => char.IsControl(character) ? ' ' : character).ToArray()).Trim();
        return text.Length <= max ? text : text[..max];
    }
}
