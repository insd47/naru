using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace Naru.Home;

/// <summary>
/// A Mac in the list: its icon, name, address, and last connection, with the actions Screen Sharing shows.
/// </summary>
/// <remarks>
/// Connect appears on hover in place of the connection time. A Mac in session always shows Disconnect, and one
/// connecting shows progress. The info button, shown only for saved Macs, opens Edit.
/// </remarks>
public sealed partial class HostRow : UserControl
{
    public static readonly DependencyProperty ItemProperty =
        DependencyProperty.Register(nameof(Item), typeof(HostItem), typeof(HostRow), new PropertyMetadata(null, OnItemChanged));

    private bool hovered;

    public HostRow()
    {
        InitializeComponent();
    }

    public HostItem Item
    {
        get => (HostItem)GetValue(ItemProperty);
        set => SetValue(ItemProperty, value);
    }

    public event Action<HostItem>? ConnectRequested;
    public event Action<HostItem>? DisconnectRequested;
    public event Action<HostItem>? InfoRequested;

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
    }

    private static void Show(UIElement element, bool visible) => element.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;

    private void OnConnect(object sender, RoutedEventArgs e) => ConnectRequested?.Invoke(Item);

    private void OnDisconnect(object sender, RoutedEventArgs e) => DisconnectRequested?.Invoke(Item);

    private void OnInfo(object sender, RoutedEventArgs e) => InfoRequested?.Invoke(Item);
}
