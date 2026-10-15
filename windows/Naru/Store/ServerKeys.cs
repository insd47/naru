using System.Text.RegularExpressions;
using Windows.Storage;

namespace Naru.Store;

/// <summary>
/// Public keys of connected Macs, saved on first connection and checked for changes on later ones.
/// </summary>
/// <remarks>
/// A Mac's public key has no certificate chain, so the first key seen is trusted and the user is asked if it later
/// changes. Keys are stored per address.
/// </remarks>
internal static partial class ServerKeys
{
    private static readonly string Folder = Path.Combine(ApplicationData.Current.LocalFolder.Path, "server-keys");

    public static byte[]? Find(Host host)
    {
        var file = FileOf(host);
        return File.Exists(file) ? File.ReadAllBytes(file) : null;
    }

    /// <summary>Saves only when no key is stored yet.</summary>
    public static void Remember(Host host, byte[] der)
    {
        var file = FileOf(host);

        if (!File.Exists(file))
        {
            Directory.CreateDirectory(Folder);
            File.WriteAllBytes(file, der);
        }
    }

    public static void Forget(Host host)
    {
        File.Delete(FileOf(host));
    }

    private static string FileOf(Host host) => Path.Combine(Folder, Unsafe().Replace(host.Address, "_") + ".der");

    // Addresses can hold IPv6 colons, which file names cannot.
    [GeneratedRegex("[^A-Za-z0-9.-]")]
    private static partial Regex Unsafe();
}
