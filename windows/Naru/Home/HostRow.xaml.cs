using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;

namespace Naru.Home;

/// <summary>
/// A Mac in the list: its icon, name, address, and last connection, with the actions Screen Sharing shows.
/// </summary>
/// <remarks>
/// Connect appears on hover in place of the connection time. A Mac in session always shows Disconnect, and one
/// connecting shows progress. The info button, shown only for saved Macs, opens Edit. Double-clicking or Enter connects,
/// or focuses the session window of a Mac in session, and Delete removes a saved Mac. A saved Mac's context menu offers
/// the same actions.
/// </remarks>
public sealed partial class HostRow : UserControl
{
    public static readonly DependencyProperty ItemProperty =
        DependencyProperty.Register(nameof(Item), typeof(HostItem), typeof(HostRow), new PropertyMetadata(null, OnItemChanged));

    private readonly MenuFlyout menu = new();
    private bool hovered;

    public HostRow()
    {
        InitializeComponent();
        menu.Opening += (_, _) => FillMenu();
    }

    public HostItem Item
    {
        get => (HostItem)GetValue(ItemProperty);
        set => SetValue(ItemProperty, value);
    }

    public event Action<HostItem>? ConnectRequested;
    public event Action<HostItem>? DisconnectRequested;
    public event Action<HostItem>? InfoRequested;
    public event Action<HostItem>? DeleteRequested;

    private static void OnItemChanged(DependencyObject target, DependencyPropertyChangedEventArgs args)
    {
        var row = (HostRow)target;

        if (args.OldValue is HostItem old)
        {
            old.PropertyChanged -= row.OnItemPropertyChanged;
        }

        if (args.NewValue is HostItem item)
        {
            item.PropertyChanged += row.OnItemPropertyChanged;
        }

        row.Refresh();
    }

    private void OnItemPropertyChanged(object? sender, PropertyChangedEventArgs args) => Refresh();

    private void OnPointerEntered(object sender, PointerRoutedEventArgs e)
    {
        hovered = true;
        Refresh();
    }

    private void OnPointerExited(object sender, PointerRoutedEventArgs e)
    {
        hovered = false;
        Refresh();
    }

    private void Refresh()
    {
        if (Item is not { } item)
        {
            return;
        }

        Show(DisconnectButton, item.InSession);
        Show(Progress, item.Connecting);
        Show(ConnectButton, item.Idle && hovered);
        Show(DetailText, item.Idle && !hovered);
        Show(InfoButton, item.Host != null);
        ContextFlyout = item.Host != null ? menu : null;

        // Hovering lightens the card as Settings cards do. Clearing the value returns to the style's theme brush.
        if (hovered)
        {
            Card.Background = (Brush)Application.Current.Resources["CardBackgroundFillColorSecondaryBrush"];
        }
        else
        {
            Card.ClearValue(Border.BackgroundProperty);
        }
    }

    private static void Show(UIElement element, bool visible) => element.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;

    // Built on each opening, since Connect and Disconnect follow the session's state.
    private void FillMenu()
    {
        menu.Items.Clear();
        menu.Items.Add(Item.InSession || Item.Connecting
            ? MenuItem("Disconnect", "", () => DisconnectRequested?.Invoke(Item))
            : MenuItem("Connect", "", () => ConnectRequested?.Invoke(Item)));
        menu.Items.Add(MenuItem("Edit", "", () => InfoRequested?.Invoke(Item)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(MenuItem("Delete", "", () => DeleteRequested?.Invoke(Item)));
    }

    private static MenuFlyoutItem MenuItem(string text, string glyph, Action action)
    {
        var item = new MenuFlyoutItem { Text = text, Icon = new FontIcon { Glyph = glyph } };
        item.Click += (_, _) => action();
        return item;
    }

    private void OnConnect(object sender, RoutedEventArgs e) => ConnectRequested?.Invoke(Item);

    private void OnDisconnect(object sender, RoutedEventArgs e) => DisconnectRequested?.Invoke(Item);

    private void OnInfo(object sender, RoutedEventArgs e) => InfoRequested?.Invoke(Item);

    private void OnDoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        ConnectRequested?.Invoke(Item);
        e.Handled = true;
    }

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            ConnectRequested?.Invoke(Item);
        }
        else if (e.Key == VirtualKey.Delete && Item.Host != null)
        {
            DeleteRequested?.Invoke(Item);
        }
        else
        {
            return;
        }

        e.Handled = true;
    }
}
