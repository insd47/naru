using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Naru.Core;
using Naru.Store;
using Windows.ApplicationModel;
using Windows.Storage;
using Windows.System;

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
        ShowCodec();
        loaded = true;
    }

    // Shows whether sessions get HEVC 4:4:4 or H.264. The check may still be running from launch.
    private async void ShowCodec()
    {
        var hevc = await Task.Run(Video.Decoder.SupportsHevc444);
        Codec.Text = hevc
            ? "Video: HEVC 4:4:4, decoded by the graphics card."
            : "Video: H.264. This PC can't decode HEVC 4:4:4 in hardware, so text may look softer.";
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

    private async void OnExport(object sender, RoutedEventArgs e)
    {
        Export.IsEnabled = false;

        try
        {
            await Diagnostics.Export(XamlRoot.ContentIslandEnvironment.AppWindowId);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or COMException)
        {
            Log.Error("Cannot export diagnostics", error);
        }
        finally
        {
            Export.IsEnabled = true;
        }
    }

    // Opens in the default text editor, as other Windows apps show their notices.
    private async void OnNotices(Hyperlink sender, HyperlinkClickEventArgs args)
    {
        try
        {
            var file = await StorageFile.GetFileFromApplicationUriAsync(new Uri("ms-appx:///ThirdPartyNotices.txt"));
            await Launcher.LaunchFileAsync(file);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or COMException)
        {
            Log.Error("Cannot open the third-party notices", error);
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
