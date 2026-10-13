using System.Collections.ObjectModel;
using System.Collections.Specialized;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Naru.Store;

namespace Naru.Home;

/// <summary>
/// Saved Macs as Settings-style cards. Each card handles its own clicks, keys, and context menu.
/// </summary>
public sealed partial class SavedPage : Page
{
    private readonly Hosts hosts = App.Current.Hosts;
    private readonly ObservableCollection<HostItem> items = [];
    private Connector connector = null!;

    public SavedPage()
    {
        InitializeComponent();
        List.ItemsSource = items;
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        connector = (Connector)e.Parameter;
        hosts.List.CollectionChanged += OnHostsChanged;
        App.Current.Sessions.Changed += OnSessionChanged;
        Rebuild();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        hosts.List.CollectionChanged -= OnHostsChanged;
        App.Current.Sessions.Changed -= OnSessionChanged;
    }

    private void OnHostsChanged(object? sender, NotifyCollectionChangedEventArgs e) => Rebuild();

    private void OnSessionChanged(Host host)
    {
        // A Mac connected from Network has its own ID until it is saved, so presence is matched by ID only.
        var item = items.FirstOrDefault(item => item.Id == host.Id);
        item?.Update(item.Host!, HostItem.PresenceOf(App.Current.Sessions.Find(host.Id)));
    }

    // Items are kept and updated in place, so focus and hover state survive changes.
    private void Rebuild()
    {
        for (var index = items.Count - 1; index >= 0; index--)
        {
            if (hosts.Find(items[index].Id) == null)
            {
                items.RemoveAt(index);
            }
        }

        for (var index = 0; index < hosts.List.Count; index++)
        {
            var host = hosts.List[index];
            var presence = HostItem.PresenceOf(App.Current.Sessions.Find(host.Id));

            if (items.FirstOrDefault(item => item.Id == host.Id) is { } existing)
            {
                existing.Update(host, presence);
            }
            else
            {
                items.Insert(Math.Min(index, items.Count), HostItem.From(host, presence));
            }
        }

        Empty.Visibility = items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnAdd(object sender, RoutedEventArgs e) => connector.Add();

    // Rows are recycled, so each one is wired once when it is first created.
    private void OnElementPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (args.Element is HostRow { Tag: null } row)
        {
            row.Tag = true;
            row.ConnectRequested += item => connector.Connect(item.Host!);
            row.DisconnectRequested += item => connector.Disconnect(item.Host!);
            row.InfoRequested += item => connector.Edit(item.Host!);
            row.DeleteRequested += item => connector.Delete(item.Host!);
        }
    }
}
