using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Naru.Home;

namespace Naru;

/// <summary>
/// The connection window, showing saved Macs, Macs found on the local network, and settings shared by every Mac.
/// </summary>
public sealed partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.SetIcon("Assets/AppIcon.ico");

        Navigation.SelectedItem = Navigation.MenuItems[0];
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
        ContentFrame.Navigate(page);
    }
}
