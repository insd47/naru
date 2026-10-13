using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Naru.Core;
using Windows.Foundation;
using Windows.Graphics;

namespace Naru.Session;

/// <summary>
/// The window of one live session, opened once authentication succeeds.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Closing the window disconnects, as in Screen Sharing.</item>
/// <item>The virtual display follows the screen area's size. The picture is centered with black bars when the sizes differ.</item>
/// <item>Pointer positions are converted to the pixels of the Mac's latest layout. Touch goes to the trackpad emulation.</item>
/// <item>Any press, wheel, or touchpad contact first ends the momentum of the last scroll.</item>
/// </list>
/// </remarks>
internal sealed partial class SessionWindow : Window
{
    // The Mac's default shortcuts for the toolbar: Control-Up Arrow, Control-Down Arrow, F11
    private const uint ControlKey = 0xffe3;
    private const uint UpKey = 0xff52;
    private const uint DownKey = 0xff54;
    private const uint F11Key = 0xffc8;

    // Windows has no system line height. Edge and Chrome scroll 100 DIPs a notch at the default 3 lines.
    private const float LineHeight = 100f / 3;

    private readonly Sessions sessions;
    private readonly Running session;
    private readonly Chrome chrome;
    private readonly Keyboard keyboard;
    private readonly Clipboard clipboard;
    private readonly MacCursor cursor;
    private readonly TouchCursor touchCursor;
    private readonly Momentum momentum;
    private readonly TouchScreen touchScreen;
    // Held so the tracker lives as long as the window
    private readonly Touchpad? touchpad;
    private readonly DispatcherQueueTimer resizing;

    // The screen area size in DIPs the window opened for, until the first layout checks it
    private (double Width, double Height)? opening;
    private bool active;
    private bool closed;

    public SessionWindow(Sessions sessions, Running session)
    {
        this.sessions = sessions;
        this.session = session;
        InitializeComponent();

        chrome = new Chrome(this, Bar, [Toolbar, Actions], CaptionInset);
        AppIcon.Apply(Icon);
        AppWindow.SetIcon("Assets/Naru.ico");
        Rename(session.Host.Title);
        Size(session.Display);

        var handle = WinRT.Interop.WindowNative.GetWindowHandle(this);
        keyboard = new Keyboard(handle, session.Connection, App.Current.Preferences, Capturing, () => FullScreen, ToggleFullScreen);
        clipboard = new Clipboard(session.Connection, App.Current.Preferences);
        cursor = new MacCursor(Screen);
        touchCursor = new TouchCursor(TouchImage);
        momentum = new Momentum(session.Connection);
        touchScreen = new TouchScreen(session.Connection, momentum, touchCursor);

        // Without the tracker, the touchpad still scrolls through the wheel handler, only in steps.
        try
        {
            touchpad = new Touchpad(Screen, session.Connection.Scroll, momentum, () => (float)(Place()?.PointsPerDip ?? 1));
        }
        catch (Exception error) when (error is COMException or InvalidOperationException)
        {
            Log.Error("Cannot take touchpad scrolling", error);
        }

        if (session.Cursor is { } shape)
        {
            cursor.Show(shape);
            touchCursor.Show(shape);
        }

        resizing = DispatcherQueue.CreateTimer();
        resizing.Interval = TimeSpan.FromMilliseconds(500);
        resizing.IsRepeating = false;
        resizing.Tick += (_, _) => ResizeDisplay();

        AppWindow.Closing += (_, _) =>
        {
            momentum.Stop();
            chrome.Close();
            sessions.Close(session.Host.Id);
        };
        AppWindow.Changed += OnChanged;
        Activated += OnActivated;
        Closed += (_, _) =>
        {
            keyboard.Dispose();
            clipboard.Dispose();
            cursor.Dispose();
        };

        session.Connection.Video.FirstFrame += () => DispatcherQueue.TryEnqueue(() => Loading.Visibility = Visibility.Collapsed);
        Surface.Loaded += (_, _) => session.Connection.Video.Attach(Surface, PixelWidth, PixelHeight, Scale);
        Surface.SizeChanged += (_, _) => OnSurfaceChanged();
        Surface.CompositionScaleChanged += (_, _) => OnSurfaceChanged();

        Screen.PointerMoved += (_, e) => OnPointer(e, TouchPhase.Moved);
        Screen.PointerPressed += (_, e) => OnPointer(e, TouchPhase.Began);
        Screen.PointerReleased += (_, e) => OnPointer(e, TouchPhase.Ended);
        Screen.PointerCanceled += OnPointerEnded;
        Screen.PointerCaptureLost += OnPointerEnded;
        Screen.PointerWheelChanged += OnWheel;
    }

    private bool FullScreen => AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen;

    // The screen area in physical pixels and the monitor scale it is drawn at
    private uint PixelWidth => (uint)Math.Round(Surface.ActualWidth * Surface.CompositionScaleX);
    private uint PixelHeight => (uint)Math.Round(Surface.ActualHeight * Surface.CompositionScaleY);
    private float Scale => Surface.CompositionScaleX;

    public void Rename(string title)
    {
        Title = title;
        Caption.Text = title;
    }

    /// <summary>A session event, on the UI thread.</summary>
    public void Handle(Event @event)
    {
        switch (@event)
        {
            case Event.Clipboard text:
                clipboard.OnServer(text.Text);
                break;
            case Event.Cursor shape:
                cursor.Show(shape);
                touchCursor.Show(shape);
                PlaceCursors();
                break;
            case Event.Layout:
                PlaceCursors();
                break;
        }
    }

    /// <summary>The session ended. The window closes without disconnecting again.</summary>
    public void Ended()
    {
        if (!closed)
        {
            closed = true;
            chrome.Close();
            Close();
        }
    }

    public void ToggleFullScreen()
    {
        AppWindow.SetPresenter(FullScreen ? AppWindowPresenterKind.Default : AppWindowPresenterKind.FullScreen);
    }

    private Placement? Place() => Placement.Of(session.Layout, PixelWidth, PixelHeight, Scale);

    // Both cursors follow the picture's placement, which changes with the layout, the window, and the monitor.
    private void PlaceCursors()
    {
        if (Place() is not { } place)
        {
            return;
        }

        cursor.Scale(place.PixelsPerPoint);
        touchScreen.Place(place);
    }

    // Keys go to the Mac while this window is in front and no menu or dialog is open over it.
    private bool Capturing() => active && Content.XamlRoot is { } root && VisualTreeHelper.GetOpenPopupsForXamlRoot(root).Count == 0;

    private void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        var now = args.WindowActivationState != WindowActivationState.Deactivated;

        if (now == active)
        {
            return;
        }

        active = now;

        if (active)
        {
            keyboard.Activated();
            clipboard.Activated();
        }
        else
        {
            keyboard.Deactivated();
            clipboard.Deactivated();
        }

        // An inactive title bar shows its title semi-transparent, as the Windows guidance asks.
        Heading.Opacity = active ? 1 : 0.6;
    }

    // The picture follows the window at once. The Mac's virtual display follows once resizing pauses, since each change makes the Mac redraw.
    private void OnSurfaceChanged()
    {
        Correct();
        session.Connection.Video.Resize(PixelWidth, PixelHeight, Scale);
        PlaceCursors();
        resizing.Stop();
        resizing.Start();
    }

    private void ResizeDisplay()
    {
        if (Surface.ActualWidth > 0 && Surface.ActualHeight > 0)
        {
            sessions.Resize(session, Displays.Mode(Surface.ActualWidth, Surface.ActualHeight, Scale));
        }
    }

    // The client area fits the virtual display below the title bar. Without one, a size that suits the screen is used.
    private void Size(DisplayMode? display)
    {
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var scale = Displays.Scale(AppWindow);
        var (width, height) = display is { } mode ? ((int)mode.Width, (int)mode.Height) : Displays.Default(area, scale);
        opening = (width, height);
        AppWindow.ResizeClient(new SizeInt32((int)(width * scale), (int)((height + Bar.Height) * scale)));
        Center();
    }

    // ResizeClient leaves out the system caption, which the extended content covers too, so the first layout comes out
    // taller than asked. The window is corrected by the measured difference once, so the Mac's display need not follow.
    private void Correct()
    {
        if (opening is not { } target || Surface.ActualWidth == 0)
        {
            return;
        }

        opening = null;
        var width = (int)Math.Round((target.Width - Surface.ActualWidth) * Scale);
        var height = (int)Math.Round((target.Height - Surface.ActualHeight) * Scale);

        if (width != 0 || height != 0)
        {
            AppWindow.Resize(new SizeInt32(AppWindow.Size.Width + width, AppWindow.Size.Height + height));
            Center();
        }
    }

    private void Center()
    {
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        AppWindow.Move(new PointInt32(
            area.X + Math.Max(0, (area.Width - AppWindow.Size.Width) / 2),
            area.Y + Math.Max(0, (area.Height - AppWindow.Size.Height) / 2)));
    }

    // An exception escaping an AppWindow handler makes the windowing layer end the process, so it is logged instead.
    private void OnChanged(AppWindow sender, AppWindowChangedEventArgs args)
    {
        try
        {
            chrome.Follow(args);

            // In full screen the screen spans both rows, under the title bar's overlay.
            if (args.DidPresenterChange)
            {
                var full = FullScreen;
                Grid.SetRow(Screen, full ? 0 : 1);
                Grid.SetRowSpan(Screen, full ? 2 : 1);
                FullScreenIcon.Glyph = full ? "" : "";
            }
        }
        catch (Exception error)
        {
            Log.Error("Cannot follow a window change", error);
        }
    }

    // The phase names what happened for mouse and pen too: Began is a press and Ended a release.
    private void OnPointer(PointerRoutedEventArgs e, TouchPhase phase)
    {
        var point = e.GetCurrentPoint(Surface);
        chrome.Hover(point.Position, point.IsInContact);

        if (Place() is not { } place)
        {
            return;
        }

        if (phase == TouchPhase.Began)
        {
            momentum.Stop();
        }

        if (e.Pointer.PointerDeviceType == PointerDeviceType.Touch)
        {
            touchScreen.Touch(point, e.GetIntermediatePoints(Surface), phase, place);
        }
        else
        {
            var mac = place.Mac(point.Position);
            touchScreen.Follow(mac);

            var properties = point.Properties;
            var buttons = (byte)((properties.IsLeftButtonPressed ? 1 : 0) | (properties.IsRightButtonPressed ? 2 : 0) | (properties.IsMiddleButtonPressed ? 4 : 0));
            session.Connection.Pointer(buttons, (ushort)mac.X, (ushort)mac.Y);
        }

        // Capture keeps a drag going when it leaves the window.
        if (phase == TouchPhase.Began)
        {
            Screen.CapturePointer(e.Pointer);
        }

        e.Handled = true;
    }

    private void OnPointerEnded(object sender, PointerRoutedEventArgs e)
    {
        if (e.Pointer.PointerDeviceType == PointerDeviceType.Touch && Place() is { } place)
        {
            touchScreen.Touch(e.GetCurrentPoint(Surface), [], TouchPhase.Cancelled, place);
        }
    }

    // A notch is 120 and scrolls the lines the Windows setting says, read each time so a change applies at once. There
    // is no acceleration, as with a wheel on a Mac. Windows counts up and right as positive, the session up and left.
    private void OnWheel(object sender, PointerRoutedEventArgs e)
    {
        momentum.Stop();
        var properties = e.GetCurrentPoint(Surface).Properties;
        var notches = properties.MouseWheelDelta / 120f;
        var points = LineHeight * (float)(Place()?.PointsPerDip ?? 1);

        if (properties.IsHorizontalMouseWheel)
        {
            var lines = -notches * Native.WheelLines(Native.SpiGetWheelScrollChars);
            session.Connection.Wheel(lines * points, 0, lines, 0);
        }
        else
        {
            var lines = notches * Native.WheelLines(Native.SpiGetWheelScrollLines);
            session.Connection.Wheel(0, lines * points, 0, lines);
        }

        e.Handled = true;
    }

    private void OnMissionControl(object sender, RoutedEventArgs e) => Press(ControlKey, UpKey);

    private void OnAppWindows(object sender, RoutedEventArgs e) => Press(ControlKey, DownKey);

    private void OnDesktop(object sender, RoutedEventArgs e) => Press(F11Key);

    private void OnFullScreen(object sender, RoutedEventArgs e) => ToggleFullScreen();

    private void OnShowConnections(object sender, RoutedEventArgs e) => App.Current.ShowConnections();

    private void OnDisconnect(object sender, RoutedEventArgs e) => sessions.Close(session.Host.Id);

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
