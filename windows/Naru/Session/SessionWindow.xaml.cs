using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Naru.Core;
using Windows.Graphics;

namespace Naru.Session;

/// <summary>
/// The window of one live session, opened once authentication succeeds.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Closing the window disconnects, as in Screen Sharing.</item>
/// <item>The title bar holds the toolbar. In full screen it hides and slides back over the screen from the top edge.</item>
/// </list>
/// </remarks>
internal sealed partial class SessionWindow : Window
{
    private readonly Sessions sessions;
    private readonly Running session;
    private bool ended;

    public SessionWindow(Sessions sessions, Running session)
    {
        this.sessions = sessions;
        this.session = session;
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(Bar);
        AppWindow.SetIcon("Assets/Naru.ico");
        Rename(session.Host.Title);
        Size(session.Display);

        AppWindow.Closing += (_, _) => sessions.Close(session.Host.Id);
        AppWindow.Changed += OnChanged;
    }

    public void Rename(string title)
    {
        Title = title;
        Bar.Title = title;
    }

    /// <summary>A session event, on the UI thread.</summary>
    public void Handle(Event @event)
    {
    }

    /// <summary>The session ended. The window closes without disconnecting again.</summary>
    public void Ended()
    {
        if (!ended)
        {
            ended = true;
            Close();
        }
    }

    // The client area fits the virtual display below the title bar. Without one, a size that suits the screen is used.
    private void Size(DisplayMode? display)
    {
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var scale = Displays.Scale(AppWindow);
        var (width, height) = display is { } mode ? ((int)mode.Width, (int)mode.Height) : Displays.Default(area, scale);
        var client = new SizeInt32((int)(width * scale), (int)((height + Bar.Height) * scale));
        AppWindow.ResizeClient(client);
        AppWindow.Move(new PointInt32(
            area.X + Math.Max(0, (area.Width - AppWindow.Size.Width) / 2),
            area.Y + Math.Max(0, (area.Height - AppWindow.Size.Height) / 2)));
    }

    private void OnChanged(AppWindow sender, AppWindowChangedEventArgs args)
    {
        if (args.DidPresenterChange)
        {
            var full = sender.Presenter.Kind == AppWindowPresenterKind.FullScreen;
            Bar.Visibility = full ? Visibility.Collapsed : Visibility.Visible;
            Grid.SetRow(Screen, full ? 0 : 1);
            Grid.SetRowSpan(Screen, full ? 2 : 1);
            FullScreenIcon.Glyph = full ? "" : "";
        }
    }

    // The Mac's default shortcuts: Control-Up Arrow, Control-Down Arrow, F11
    private void OnMissionControl(object sender, RoutedEventArgs e) => Press(Keysyms.ControlL, Keysyms.Up);

    private void OnAppWindows(object sender, RoutedEventArgs e) => Press(Keysyms.ControlL, Keysyms.Down);

    private void OnDesktop(object sender, RoutedEventArgs e) => Press(Keysyms.F11);

    private void OnFullScreen(object sender, RoutedEventArgs e) => ToggleFullScreen();

    public void ToggleFullScreen()
    {
        var full = AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen;
        AppWindow.SetPresenter(full ? AppWindowPresenterKind.Default : AppWindowPresenterKind.FullScreen);
    }

    private void OnShowConnections(object sender, RoutedEventArgs e) => App.Current.ShowConnections();

    private void OnDisconnect(object sender, RoutedEventArgs e) => sessions.Close(session.Host.Id);

    // Presses the keys in order and releases them in reverse.
    private void Press(params uint[] keysyms)
    {
        foreach (var keysym in keysyms)
        {
            session.Connection.Key(keysym, true);
        }

        foreach (var keysym in keysyms.Reverse())
        {
            session.Connection.Key(keysym, false);
        }
    }
}
