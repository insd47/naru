using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Naru.Core;
using Naru.Session;
using Naru.Store;

namespace Naru.Home;

/// <summary>
/// Connects to Macs from the connection window: asks for credentials, starts sessions, and reports failures.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>A saved password skips the sign-in dialog. Otherwise the dialog stays open while connecting, and closes once authentication succeeds.</item>
/// <item>A wrong password shows in the sign-in dialog. Other failures show in their own dialog with a way to reconnect.</item>
/// <item>As in Screen Sharing, the session window opens only after authentication succeeds.</item>
/// </list>
/// </remarks>
internal sealed class Connector
{
    private readonly Func<XamlRoot> root;
    private readonly AppWindow window;
    private readonly Hosts hosts = App.Current.Hosts;
    private readonly Sessions sessions = App.Current.Sessions;

    // The sign-in dialog waiting for this Mac's outcome: null on success, or the failure.
    private (string HostId, TaskCompletionSource<Failure?> Outcome)? pending;

    public Connector(Func<XamlRoot> root, AppWindow window)
    {
        this.root = root;
        this.window = window;
        sessions.Changed += OnChanged;
        sessions.Failed += Report;
    }

    public void Detach()
    {
        sessions.Changed -= OnChanged;
        sessions.Failed -= Report;
    }

    /// <summary>Focuses the session window of a Mac in session, or connects with the saved password, or asks for one.</summary>
    public async void Connect(Host host)
    {
        if (sessions.Find(host.Id) is { } running)
        {
            running.Window?.Activate();
            return;
        }

        var password = host.Username.Length > 0 ? Keychain.Find(host.Id) : null;

        if (password == null)
        {
            await SignIn(host, null);
            return;
        }

        Open(host, new Credentials(host.Username, password, Remember: false));
    }

    /// <summary>Connects to a Mac found on the network, using a saved Mac with the same address if there is one.</summary>
    public void Connect(Found found)
    {
        if (found.Address is not { } address)
        {
            return;
        }

        var saved = hosts.List.FirstOrDefault(host => host.Address == address && host.Port == found.Port);
        Connect(saved ?? new Host { Name = found.Name, Address = address, Port = found.Port });
    }

    /// <summary>Asks for an address and connects. The Mac is saved to the list once authentication succeeds.</summary>
    public async void Add()
    {
        if (await Dialogs.ConnectTo(root()) is { } address)
        {
            Connect(new Host { Address = address });
        }
    }

    public void Disconnect(Host host) => sessions.Close(host.Id);

    // Only the edited fields are applied to the latest entry, so a connection time or name a session saved while the dialog was open is kept.
    public async void Edit(Host host)
    {
        if (await Dialogs.Edit(root(), host) is not { } edited)
        {
            return;
        }

        var latest = hosts.Find(edited.Id) ?? edited;
        hosts.Save(latest with
        {
            Name = edited.Name,
            Address = edited.Address,
            Port = edited.Port,
            Username = edited.Username,
            VirtualDisplay = edited.VirtualDisplay,
        });
    }

    // Server keys are stored per address, so the key stays while another saved Mac uses the same address.
    public void Delete(Host host)
    {
        sessions.Close(host.Id);
        hosts.Remove(host);
        Keychain.Remove(host.Id);

        if (hosts.List.All(other => other.Address != host.Address))
        {
            ServerKeys.Forget(host);
        }
    }

    private void Open(Host host, Credentials credentials)
    {
        sessions.Open(host, credentials, host.VirtualDisplay ? Displays.Initial(window) : null);
    }

    private async Task SignIn(Host host, string? message)
    {
        var dialog = Dialogs.Base(root(), $"Sign In to “{host.Title}”", "Sign In");
        var username = new TextBox { Header = "User Name", Text = host.Username };
        var password = new PasswordBox { Header = "Password" };
        var remember = new CheckBox { Content = "Remember password", IsChecked = true };
        var error = new TextBlock
        {
            Text = message ?? "",
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["SystemFillColorCriticalBrush"],
            Visibility = message == null ? Visibility.Collapsed : Visibility.Visible,
            TextWrapping = TextWrapping.Wrap,
        };

        dialog.Content = new StackPanel
        {
            Spacing = 12,
            MinWidth = 360,
            Children =
            {
                new TextBlock { Text = "Enter your name and password to share this Mac's screen.", TextWrapping = TextWrapping.Wrap },
                username,
                password,
                error,
                remember,
            },
        };

        void Validate() => dialog.IsPrimaryButtonEnabled = username.Text.Trim().Length > 0 && password.Password.Length > 0;
        username.TextChanged += (_, _) => Validate();
        password.PasswordChanged += (_, _) => Validate();
        Validate();

        // Focus the first empty field, as Screen Sharing does.
        dialog.Opened += (_, _) => (username.Text.Length == 0 ? (Control)username : password).Focus(FocusState.Programmatic);

        void Busy(bool busy)
        {
            username.IsEnabled = password.IsEnabled = remember.IsEnabled = !busy;
            dialog.IsPrimaryButtonEnabled = !busy;
            dialog.PrimaryButtonText = busy ? "Signing In…" : "Sign In";
        }

        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            var outcome = new TaskCompletionSource<Failure?>();
            pending = (host.Id, outcome);
            error.Visibility = Visibility.Collapsed;
            Busy(true);

            Open(host, new Credentials(username.Text.Trim(), password.Password, remember.IsChecked == true));
            var failure = await outcome.Task;
            pending = null;
            Busy(false);

            if (failure is Failure.Rejected)
            {
                error.Text = Message(failure, host);
                error.Visibility = Visibility.Visible;
                password.Password = "";
                password.Focus(FocusState.Programmatic);
                args.Cancel = true;
            }

            deferral.Complete();
        };

        dialog.CloseButtonClick += (_, _) =>
        {
            if (pending?.HostId == host.Id)
            {
                sessions.Close(host.Id);
            }
        };

        await Dialogs.Show(dialog);
    }

    private void OnChanged(Host host)
    {
        if (pending is { } waiting && waiting.HostId == host.Id && sessions.Find(host.Id)?.Authenticated == true)
        {
            waiting.Outcome.TrySetResult(null);
        }
    }

    /// <summary>Shows a failure. A failure during sign-in goes to the waiting dialog, and a wrong password without one asks again.</summary>
    public async void Report(Host host, Failure failure)
    {
        if (pending is { } waiting && waiting.HostId == host.Id)
        {
            waiting.Outcome.TrySetResult(failure);

            if (failure is Failure.Rejected)
            {
                return;
            }
        }
        else if (failure is Failure.Rejected)
        {
            await SignIn(host, Message(failure, host));
            return;
        }

        var keyChanged = failure is Failure.KeyChanged;
        var dialog = Dialogs.Base(root(), keyChanged ? "Server Identity Changed" : "Connection Failed", keyChanged ? "Trust and Reconnect" : "Reconnect", "Close");
        dialog.Content = new TextBlock { Text = Message(failure, host), TextWrapping = TextWrapping.Wrap, MaxWidth = 400 };
        dialog.DefaultButton = keyChanged ? ContentDialogButton.Close : ContentDialogButton.Primary;

        if (await Dialogs.Show(dialog) != ContentDialogResult.Primary)
        {
            return;
        }

        if (keyChanged)
        {
            ServerKeys.Forget(host);
        }

        Connect(hosts.Find(host.Id) ?? host);
    }

    private static string Message(Failure failure, Host host) => failure switch
    {
        Failure.Rejected => "Incorrect name or password",
        Failure.KeyChanged => "This Mac's identity is different from the last time you connected. Reconnect only if you trust this Mac.",
        Failure.OtherUser other => $"{other.Name} is logged in to this Mac.",
        Failure.Unreachable => $"Can't reach {host.Address}. Check the address and that Screen Sharing is turned on.",
        Failure.Unsupported => "This Mac doesn't support High Performance screen sharing.",
        Failure.NoMedia => "The Mac didn't start sending its screen.",
        _ => "The connection ended unexpectedly.",
    };
}
