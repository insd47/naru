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
        Hosts = new Hosts();
        Preferences = new Preferences();
        Sessions = new Sessions(Hosts, Preferences);
        ShowConnections();
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
