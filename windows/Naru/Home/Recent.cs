using System.Runtime.InteropServices;
using Naru.Store;
using Windows.UI.StartScreen;

namespace Naru.Home;

/// <summary>
/// Keeps the taskbar jump list's Recent group in step with the Macs connected to most recently.
/// </summary>
/// <remarks>
/// An item launches the app with <c>connect:</c> and the Mac's ID, which <see cref="HostId"/> reads back. The running
/// app receives it through activation redirection, so choosing an item never opens a second instance.
/// </remarks>
internal static class Recent
{
    private const string Prefix = "connect:";
    private const int Count = 5;

    /// <summary>Rebuilds the list from the saved Macs. Failures only cost the jump list, so they are logged.</summary>
    public static async void Update(IEnumerable<Host> hosts)
    {
        if (!JumpList.IsSupported())
        {
            return;
        }

        try
        {
            var list = await JumpList.LoadCurrentAsync();
            list.SystemGroupKind = JumpListSystemGroupKind.None;
            list.Items.Clear();

            foreach (var host in hosts.Where(host => host.LastConnected != null).OrderByDescending(host => host.LastConnected).Take(Count))
            {
                var item = JumpListItem.CreateWithArguments(Prefix + host.Id, host.Title);
                item.GroupName = "Recent";
                item.Description = host.Address;
                item.Logo = new Uri("ms-appx:///Assets/Square44x44Logo.png");
                list.Items.Add(item);
            }

            await list.SaveAsync();
        }
        catch (COMException error)
        {
            Log.Error("Cannot update the jump list", error);
        }
    }

    /// <summary>The Mac a jump list item names in its launch arguments, if they came from one.</summary>
    public static string? HostId(string? arguments) =>
        arguments is { } text && text.StartsWith(Prefix, StringComparison.Ordinal) ? text[Prefix.Length..] : null;
}
