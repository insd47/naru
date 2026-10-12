using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Naru.Store;

namespace Naru.Home;

/// <summary>
/// The connection window's dialogs.
/// </summary>
/// <remarks>A window can show only one ContentDialog at a time, so each waits for the one before it to close.</remarks>
internal static class Dialogs
{
    private static readonly SemaphoreSlim Showing = new(1, 1);

    public static async Task<ContentDialogResult> Show(ContentDialog dialog)
    {
        await Showing.WaitAsync();

        try
        {
            return await dialog.ShowAsync();
        }
        finally
        {
            Showing.Release();
        }
    }

    public static ContentDialog Base(XamlRoot root, string title, string primary, string close = "Cancel") => new()
    {
        XamlRoot = root,
        Style = (Style)Application.Current.Resources["DefaultContentDialogStyle"],
        Title = title,
        PrimaryButtonText = primary,
        CloseButtonText = close,
        DefaultButton = ContentDialogButton.Primary,
    };

    /// <summary>A new connection asks only for the address, as in Screen Sharing.</summary>
    public static async Task<string?> ConnectTo(XamlRoot root)
    {
        var dialog = Base(root, "Connect To", "Connect");
        var address = new TextBox { Header = "Server Address", PlaceholderText = "Hostname or IP address", MinWidth = 320 };
        dialog.Content = address;
        dialog.IsPrimaryButtonEnabled = false;
        address.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = address.Text.Trim().Length > 0;

        return await Show(dialog) == ContentDialogResult.Primary ? address.Text.Trim() : null;
    }

    /// <summary>Edits a saved Mac's name, address, username, and display. An empty name is filled with the Mac's computer name on the next connection.</summary>
    public static async Task<Host?> Edit(XamlRoot root, Host original)
    {
        var dialog = Base(root, $"Edit “{original.Title}”", "Save");
        var name = new TextBox { Header = "Name", Text = original.Name, PlaceholderText = "Filled in when you connect" };
        var address = new TextBox { Header = "Server Address", Text = original.Address };
        var port = new NumberBox { Header = "Port", Value = original.Port, Minimum = 1, Maximum = 65535, Width = 112 };
        var username = new TextBox { Header = "User Name", Text = original.Username };
        var display = new RadioButtons { Header = "Display", Items = { "Virtual Display", "Mac's Display" }, SelectedIndex = original.VirtualDisplay ? 0 : 1 };

        var server = new Grid { ColumnSpacing = 12, ColumnDefinitions = { new() { Width = new GridLength(1, GridUnitType.Star) }, new() { Width = GridLength.Auto } } };
        server.Children.Add(address);
        server.Children.Add(port);
        Grid.SetColumn(port, 1);

        var fields = new StackPanel { Spacing = 16, MinWidth = 360, Children = { name, server, username, display } };

        if (original.LastConnected is { } last)
        {
            fields.Children.Add(new TextBlock
            {
                Text = $"Last connected {HostItem.FormatDate(last)}",
                Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"],
                Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
            });
        }

        dialog.Content = new ScrollViewer { Content = fields };

        void Validate() => dialog.IsPrimaryButtonEnabled = address.Text.Trim().Length > 0 && port.Value is >= 1 and <= 65535;
        address.TextChanged += (_, _) => Validate();
        port.ValueChanged += (_, _) => Validate();

        if (await Show(dialog) != ContentDialogResult.Primary)
        {
            return null;
        }

        return original with
        {
            Name = name.Text.Trim(),
            Address = address.Text.Trim(),
            Port = (int)port.Value,
            Username = username.Text.Trim(),
            VirtualDisplay = display.SelectedIndex == 0,
        };
    }
}
