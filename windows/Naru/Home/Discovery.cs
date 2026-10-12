using System.Collections.ObjectModel;
using System.Net;
using System.Net.Sockets;
using Microsoft.UI.Dispatching;
using Windows.Devices.Enumeration;

namespace Naru.Home;

/// <summary>A Screen Sharing Mac found on the local network.</summary>
public sealed record Found(string Id, string Name, string? Address, int Port);

/// <summary>
/// Finds Screen Sharing Macs on the local network over Bonjour (<c>_rfb._tcp</c>).
/// </summary>
/// <remarks>
/// Windows exposes DNS-SD results as association endpoint services, which already carry the resolved address and port.
/// Discovery runs only between <see cref="Start"/> and <see cref="Stop"/>, and <see cref="List"/> changes on the UI thread.
/// </remarks>
public sealed class Discovery
{
    // The DNS-SD protocol of the association endpoint enumeration
    private const string Selector =
        "System.Devices.AepService.ProtocolId:=\"{4526e8c1-8aac-4153-9b16-55e86ada0e54}\" AND " +
        "System.Devices.Dnssd.Domain:=\"local\" AND System.Devices.Dnssd.ServiceName:=\"_rfb._tcp\"";

    private static readonly string[] Properties =
    [
        "System.Devices.Dnssd.InstanceName",
        "System.Devices.Dnssd.PortNumber",
        "System.Devices.IpAddress",
    ];

    private readonly DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
    private DeviceWatcher? watcher;

    public ObservableCollection<Found> List { get; } = [];

    public void Start()
    {
        if (watcher != null)
        {
            return;
        }

        var next = DeviceInformation.CreateWatcher(Selector, Properties, DeviceInformationKind.AssociationEndpointService);
        next.Added += (sender, info) => Post(sender, () => Upsert(Read(info.Id, info.Properties)));
        next.Updated += (sender, update) => Post(sender, () => Update(update));
        next.Removed += (sender, update) => Post(sender, () => Remove(update.Id));
        watcher = next;
        next.Start();
    }

    public void Stop()
    {
        if (watcher is not { } current)
        {
            return;
        }

        watcher = null;

        if (current.Status is DeviceWatcherStatus.Started or DeviceWatcherStatus.EnumerationCompleted)
        {
            current.Stop();
        }

        List.Clear();
    }

    // Watcher events arrive on a background thread. Events from a stopped watcher are dropped.
    private void Post(DeviceWatcher sender, Action action)
    {
        queue.TryEnqueue(() =>
        {
            if (sender == watcher)
            {
                action();
            }
        });
    }

    private void Update(DeviceInformationUpdate update)
    {
        var index = IndexOf(update.Id);

        if (index < 0)
        {
            return;
        }

        var current = List[index];
        var changed = Read(update.Id, update.Properties);
        List[index] = current with
        {
            Name = changed.Name.Length > 0 ? changed.Name : current.Name,
            Address = changed.Address ?? current.Address,
            Port = update.Properties.ContainsKey("System.Devices.Dnssd.PortNumber") ? changed.Port : current.Port,
        };
    }

    private void Upsert(Found found)
    {
        var index = IndexOf(found.Id);

        if (index >= 0)
        {
            List[index] = found;
        }
        else
        {
            List.Add(found);
        }
    }

    private void Remove(string id)
    {
        var index = IndexOf(id);

        if (index >= 0)
        {
            List.RemoveAt(index);
        }
    }

    private int IndexOf(string id)
    {
        for (var index = 0; index < List.Count; index++)
        {
            if (List[index].Id == id)
            {
                return index;
            }
        }

        return -1;
    }

    // Prefers an IPv4 address, as the other apps do. The port defaults to Screen Sharing's.
    private static Found Read(string id, IReadOnlyDictionary<string, object> properties)
    {
        var name = properties.TryGetValue("System.Devices.Dnssd.InstanceName", out var instance) ? instance as string ?? "" : "";
        var port = properties.TryGetValue("System.Devices.Dnssd.PortNumber", out var number) && number is ushort value ? value : Store.Host.DefaultPort;
        var addresses = properties.TryGetValue("System.Devices.IpAddress", out var list) ? list as string[] ?? [] : [];
        var address = addresses
            .Select(text => IPAddress.TryParse(text, out var ip) ? ip : null)
            .OfType<IPAddress>()
            .OrderBy(ip => ip.AddressFamily == AddressFamily.InterNetwork ? 0 : 1)
            .Select(ip => ip.ToString())
            .FirstOrDefault();

        return new Found(id, name, address, port);
    }
}
