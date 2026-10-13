using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Naru.Core;
using Naru.Store;
using Windows.ApplicationModel;

namespace Naru;

/// <summary>Settings shared by every Mac.</summary>
public sealed partial class SettingsPage : Page
{
    private readonly Preferences preferences = App.Current.Preferences;
    private readonly bool loaded;

    public SettingsPage()
    {
        InitializeComponent();

        Swap.IsOn = preferences.SwapCommandOption;
        SyncLanguage.IsOn = preferences.SyncKeyboardLanguage;
        WindowsKeys.SelectedIndex = (int)preferences.WindowsKeys;
        Clipboard.IsOn = preferences.SharedClipboard;
        Quality.SelectedIndex = preferences.Priority == Priority.Quality ? 1 : 0;

        var version = Package.Current.Id.Version;
        Version.Text = $"Version {version.Major}.{version.Minor}.{version.Build}";
        ShowHevc();
        loaded = true;
    }

    // Shows whether sessions get HEVC 4:4:4 or H.264. The check may still be running from launch.
    private async void ShowHevc()
    {
        var supported = await Task.Run(Video.Decoder.SupportsHevc444);
        Hevc.Text = $"HEVC 4:4:4 hardware decoding: {(supported ? "supported" : "unsupported")}";
    }

    // Controls raise change events while the page sets their initial values, which must not write back.
    private void OnSwap(object sender, RoutedEventArgs e)
    {
        if (loaded)
        {
            preferences.SwapCommandOption = Swap.IsOn;
        }
    }

    private void OnSyncLanguage(object sender, RoutedEventArgs e)
    {
        if (loaded)
        {
            preferences.SyncKeyboardLanguage = SyncLanguage.IsOn;
        }
    }

    private void OnWindowsKeys(object sender, SelectionChangedEventArgs e)
    {
        if (loaded)
        {
            preferences.WindowsKeys = (WindowsKeys)WindowsKeys.SelectedIndex;
        }
    }

    private void OnClipboard(object sender, RoutedEventArgs e)
    {
        if (loaded)
        {
            preferences.SharedClipboard = Clipboard.IsOn;
        }
    }

    private void OnQuality(object sender, SelectionChangedEventArgs e)
    {
        if (loaded)
        {
            preferences.Priority = Quality.SelectedIndex == 1 ? Priority.Quality : Priority.Latency;
        }
    }
}
