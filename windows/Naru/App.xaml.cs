using Microsoft.UI.Xaml;
using Naru.Session;
using Naru.Store;

namespace Naru;

public partial class App : Application
{
    private MainWindow? window;

    public static new App Current => (App)Application.Current;

    internal Hosts Hosts { get; private set; } = null!;
    internal Preferences Preferences { get; private set; } = null!;
    internal Sessions Sessions { get; private set; } = null!;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        Log.ToFile();
        Hosts = new Hosts();
        Preferences = new Preferences();
        Sessions = new Sessions(Hosts, Preferences);
        Sessions.Failed += OnFailed;
        // The check opens a Direct3D device and a decoder, so it runs once in the background rather than on the first connect.
        _ = Task.Run(Video.Decoder.SupportsHevc444);

        Home.Recent.Update(Hosts.List);
        Hosts.List.CollectionChanged += (_, _) => Home.Recent.Update(Hosts.List);

        // Later launches, such as from the jump list, arrive here from their own process.
        var queue = Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread();
        Microsoft.Windows.AppLifecycle.AppInstance.GetCurrent().Activated += (_, activation) =>
        {
            var arguments = (activation.Data as Windows.ApplicationModel.Activation.ILaunchActivatedEventArgs)?.Arguments;
            queue.TryEnqueue(() => Open(arguments));
        };

        Open(args.Arguments);
    }

    // Shows the connection window, and connects to the Mac a jump list item named.
    private void Open(string? arguments)
    {
        ShowConnections();

        if (Home.Recent.HostId(arguments) is { } id && Hosts.Find(id) is { } host)
        {
            window!.Connector.Connect(host);
        }
    }

    // The connection window reports failures while it is open. Without it, it reopens to report this one.
    private void OnFailed(Host host, Core.Failure failure)
    {
        if (window == null)
        {
            ShowConnections();
            window!.Connector.Report(host, failure);
        }
    }

    /// <summary>Brings the connection window forward, reopening it if it was closed.</summary>
    public void ShowConnections()
    {
        if (window == null)
        {
            window = new MainWindow();
            window.Closed += (_, _) => window = null;
        }

        window.Activate();
    }
}
