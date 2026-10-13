using System.Collections.ObjectModel;
using System.Collections.Specialized;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Naru.Store;

namespace Naru.Home;

/// <summary>
/// Macs found on the local network. Discovery runs only while this page is shown.
/// </summary>
public sealed partial class NetworkPage : Page
{
    private readonly Discovery discovery = new();
    private readonly ObservableCollection<HostItem> items = [];
    private Connector connector = null!;

    public NetworkPage()
    {
        InitializeComponent();
        List.ItemsSource = items;
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        connector = (Connector)e.Parameter;
        discovery.List.CollectionChanged += OnFoundChanged;
        App.Current.Sessions.Changed += OnSessionChanged;
        discovery.Start();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        discovery.Stop();
        discovery.List.CollectionChanged -= OnFoundChanged;
        App.Current.Sessions.Changed -= OnSessionChanged;
    }

    private void OnFoundChanged(object? sender, NotifyCollectionChangedEventArgs e) => Rebuild();

    private void OnSessionChanged(Host host) => Rebuild();

    private void Rebuild()
    {
        for (var index = items.Count - 1; index >= 0; index--)
        {
            if (discovery.List.All(found => found.Id != items[index].Id))
            {
                items.RemoveAt(index);
            }
        }

        foreach (var found in discovery.List)
        {
            var presence = PresenceOf(found);

            if (items.FirstOrDefault(item => item.Id == found.Id) is { } existing)
            {
                existing.Found = found;
                existing.Title = found.Name;
                existing.Presence = presence;
            }
            else
            {
                items.Add(HostItem.From(found, presence));
            }
        }

        Empty.Visibility = items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    // A found Mac has no saved ID, so its session is matched by address.
    private static Presence PresenceOf(Found found) => App.Current.Sessions.FindAddress(found.Address, found.Port) switch
    {
        null => Presence.Idle,
        { Authenticated: true } => Presence.InSession,
        _ => Presence.Connecting,
    };

    // Rows are recycled, so each one is wired once when it is first created.
    private void OnElementPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (args.Element is HostRow { Tag: null } row)
        {
            row.Tag = true;
            row.ConnectRequested += item => connector.Connect(item.Found!);
            row.DisconnectRequested += Disconnect;
        }
    }

    private void Disconnect(HostItem item)
    {
        if (App.Current.Sessions.FindAddress(item.Found!.Address, item.Found.Port) is { } running)
        {
            connector.Disconnect(running.Host);
        }
    }

    private void OnAdd(object sender, RoutedEventArgs e) => connector.Add();
}
