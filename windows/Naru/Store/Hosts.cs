using System.Collections.ObjectModel;
using System.Text.Json;
using System.Text.Json.Serialization;
using Windows.Storage;

namespace Naru.Store;

/// <summary>A saved Mac. <see cref="Keychain"/> keeps the password separately.</summary>
public sealed record Host
{
    public const int DefaultPort = 5900;

    public string Id { get; init; } = Guid.NewGuid().ToString();

    /// <summary>Name given by the user. If empty, it is filled with the computer name the Mac reports on connect.</summary>
    public string Name { get; init; } = "";

    public required string Address { get; init; }
    public string Username { get; init; } = "";
    public int Port { get; init; } = DefaultPort;

    /// <summary>Whether to connect with a virtual display sized to the window. When off, the Mac's real display is shown.</summary>
    public bool VirtualDisplay { get; init; } = true;

    public DateTimeOffset? LastConnected { get; init; }

    /// <summary>Name to display. Falls back to the address when there is no name.</summary>
    [JsonIgnore]
    public string Title => Name.Length > 0 ? Name : Address;
}

/// <summary>
/// The list of saved Macs, stored as JSON in the app's local folder.
/// </summary>
/// <remarks>Modified only on the UI thread.</remarks>
internal sealed class Hosts
{
    private readonly string path = Path.Combine(ApplicationData.Current.LocalFolder.Path, "hosts.json");

    public ObservableCollection<Host> List { get; }

    public Hosts()
    {
        List = new ObservableCollection<Host>(Load());
    }

    public Host? Find(string id) => List.FirstOrDefault(host => host.Id == id);

    /// <summary>Replaces the entry with the same ID, or appends it if there is none.</summary>
    public void Save(Host host)
    {
        var index = IndexOf(host.Id);

        if (index >= 0)
        {
            List[index] = host;
        }
        else
        {
            List.Add(host);
        }

        Store();
    }

    public void Remove(Host host)
    {
        var index = IndexOf(host.Id);

        if (index >= 0)
        {
            List.RemoveAt(index);
            Store();
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

    private List<Host> Load()
    {
        try
        {
            return File.Exists(path) ? JsonSerializer.Deserialize(File.ReadAllText(path), StoreJson.Default.ListHost) ?? [] : [];
        }
        catch (JsonException)
        {
            // A damaged file loses the list rather than keeping the app from starting.
            return [];
        }
    }

    // Written to a temporary file and moved into place, so a crash mid-write cannot leave half a list.
    private void Store()
    {
        var temporary = path + ".tmp";
        File.WriteAllText(temporary, JsonSerializer.Serialize(List.ToList(), StoreJson.Default.ListHost));
        File.Move(temporary, path, overwrite: true);
    }
}

// Source-generated so serialization survives trimming in release builds.
[JsonSourceGenerationOptions(PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase, WriteIndented = true)]
[JsonSerializable(typeof(List<Host>))]
internal sealed partial class StoreJson : JsonSerializerContext;
