using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Windowing;
using Naru.Home;
using Naru.Session;
using Windows.Graphics;
using Windows.System;

namespace Naru;

/// <summary>
/// The connection window, showing saved Macs, Macs found on the local network, and settings shared by every Mac.
/// </summary>
/// <remarks>Sessions continue in their own windows when this one closes.</remarks>
public sealed partial class MainWindow : Window
{
    private readonly Connector connector;

    public MainWindow()
    {
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.SetIcon("Assets/Naru.ico");
        Place();

        connector = new Connector(() => Content.XamlRoot, AppWindow);
        Closed += (_, _) => connector.Detach();

        var settings = new KeyboardAccelerator { Modifiers = VirtualKeyModifiers.Control, Key = (VirtualKey)0xBC };
        settings.Invoked += (_, args) =>
        {
            Navigation.SelectedItem = Navigation.SettingsItem;
            args.Handled = true;
        };
        Navigation.KeyboardAccelerators.Add(settings);

        Navigation.SelectedItem = Navigation.MenuItems[0];
    }

    // A modest window centered in the work area, like Screen Sharing's connection window.
    private void Place()
    {
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var scale = Displays.Scale(AppWindow);
        var size = new SizeInt32(Math.Min((int)(960 * scale), area.Width), Math.Min((int)(640 * scale), area.Height));
        AppWindow.MoveAndResize(new RectInt32(area.X + (area.Width - size.Width) / 2, area.Y + (area.Height - size.Height) / 2, size.Width, size.Height));
    }

    private void OnPaneToggleRequested(TitleBar sender, object args)
    {
        Navigation.IsPaneOpen = !Navigation.IsPaneOpen;
    }

    private void OnSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        var page = args.IsSettingsSelected
            ? typeof(SettingsPage)
            : (args.SelectedItemContainer?.Tag as string) switch
            {
                "Network" => typeof(NetworkPage),
                _ => typeof(SavedPage),
            };
        ContentFrame.Navigate(page, connector);
    }
}
