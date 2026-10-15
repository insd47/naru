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

    public SettingsPage()
    {
        InitializeComponent();

        // Wired after the initial selection, which raises a change of its own.
        KeyCombinations.SelectedIndex = (int)preferences.WindowsKeys;
        KeyCombinations.SelectionChanged += (_, _) => preferences.WindowsKeys = (WindowsKeys)KeyCombinations.SelectedIndex;

        var version = Package.Current.Id.Version;
        Version.Text = $"Cross over to your Mac. Version {version.Major}.{version.Minor}.{version.Build}";
        ShowCodec();
    }

    // Shows whether sessions get HEVC 4:4:4 or H.264. The check may still be running from launch.
    private async void ShowCodec()
    {
        var hevc = await Task.Run(Video.Decoder.SupportsHevc444);
        Codec.Text = hevc
            ? "Video: HEVC 4:4:4, decoded by the graphics card."
            : "Video: H.264. This PC can't decode HEVC 4:4:4 in hardware, so text may look softer.";
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
}
