using System.Collections.ObjectModel;
using System.Collections.Specialized;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Navigation;
using Naru.Store;
using Windows.System;

namespace Naru.Home;

/// <summary>
/// Saved Macs, as a list or as icons. Double-clicking or pressing Enter connects, or focuses the session window of a Mac in session.
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
        item?.Update(item.Host!, PresenceOf(host.Id));
    }

    // Items are kept and updated in place, so the selection and hover state survive changes.
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
            var existing = items.FirstOrDefault(item => item.Id == host.Id);

            if (existing == null)
            {
                items.Insert(Math.Min(index, items.Count), HostItem.From(host, PresenceOf(host.Id)));
            }
            else
            {
                existing.Update(host, PresenceOf(host.Id));
            }
        }

        Empty.Visibility = items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private static Presence PresenceOf(string hostId) => App.Current.Sessions.Find(hostId) switch
    {
        null => Presence.Idle,
        { Authenticated: true } => Presence.InSession,
        _ => Presence.Connecting,
    };

    private void OnAdd(object sender, RoutedEventArgs e) => connector.Add();

    // Rows are recycled, so each one is wired once when it is first created.
    private void OnContainerContentChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.ItemContainer.ContentTemplateRoot is HostRow { Tag: null } row)
        {
            row.Tag = true;
            row.ConnectRequested += OnConnect;
            row.DisconnectRequested += OnDisconnect;
            row.InfoRequested += OnInfo;
        }
    }

    private void OnConnect(HostItem item) => connector.Connect(item.Host!);

    private void OnDisconnect(HostItem item) => connector.Disconnect(item.Host!);

    private void OnInfo(HostItem item) => connector.Edit(item.Host!);

    private void OnDoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        if ((e.OriginalSource as FrameworkElement)?.DataContext is HostItem item)
        {
            connector.Connect(item.Host!);
        }
    }

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (List.SelectedItem is not HostItem item)
        {
            return;
        }

        switch (e.Key)
        {
            case VirtualKey.Enter:
                connector.Connect(item.Host!);
                e.Handled = true;
                break;
            case VirtualKey.Delete:
                connector.Delete(item.Host!);
                e.Handled = true;
                break;
        }
    }

    private void OnContextRequested(UIElement sender, ContextRequestedEventArgs e)
    {
        if ((e.OriginalSource as FrameworkElement)?.DataContext is not HostItem { Host: { } host } item)
        {
            return;
        }

        var menu = new MenuFlyout();
        menu.Items.Add(item.InSession || item.Connecting
            ? Item("Disconnect", "", () => connector.Disconnect(host))
            : Item("Connect", "", () => connector.Connect(host)));
        menu.Items.Add(Item("Edit", "", () => connector.Edit(host)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Item("Delete", "", () => connector.Delete(host)));

        if (e.TryGetPosition((UIElement)e.OriginalSource, out var point))
        {
            menu.ShowAt((UIElement)e.OriginalSource, point);
        }
        else
        {
            menu.ShowAt((FrameworkElement)e.OriginalSource);
        }

        e.Handled = true;
    }

    private static MenuFlyoutItem Item(string text, string glyph, Action action)
    {
        var item = new MenuFlyoutItem { Text = text, Icon = new FontIcon { Glyph = glyph } };
        item.Click += (_, _) => action();
        return item;
    }
}
