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
/// <item>Pointer positions are converted to the pixels of the Mac's latest layout. Touch goes to the session's trackpad emulation.</item>
/// <item>The title bar holds the toolbar. In full screen it hides, and slides back over the screen when the pointer rests at the top edge.</item>
/// </list>
/// </remarks>
internal sealed partial class SessionWindow : Window
{
    // How long the pointer rests at the top edge in full screen before the title bar slides down
    private static readonly TimeSpan RevealDelay = TimeSpan.FromMilliseconds(300);

    private readonly Sessions sessions;
    private readonly Running session;
    private readonly Keyboard keyboard;
    private readonly Clipboard clipboard;
    private readonly MacCursor cursor;
    private readonly DispatcherQueueTimer resizing;
    private readonly DispatcherQueueTimer revealing;
    private RectInt32[] passthrough = [];
    private bool applying;
    private bool active;
    private bool ended;

    // Set as soon as the window starts closing. Layout may run once more while it goes away, when its title bar
    // regions can no longer be set.
    private bool closed;

    public SessionWindow(Sessions sessions, Running session)
    {
        this.sessions = sessions;
        this.session = session;
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(Bar);
        // Tall caption buttons match the 48 px bar, so they line up with its buttons.
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppIcon.Apply(Icon);
        AppWindow.SetIcon("Assets/Naru.ico");
        Rename(session.Host.Title);
        Size(session.Display);

        var handle = WinRT.Interop.WindowNative.GetWindowHandle(this);
        keyboard = new Keyboard(handle, session.Connection, App.Current.Preferences, Capturing, () => FullScreen, ToggleFullScreen);
        clipboard = new Clipboard(session.Connection, App.Current.Preferences);
        cursor = new MacCursor(Screen);

        AppWindow.Closing += (_, _) =>
        {
            closed = true;
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

        resizing = DispatcherQueue.CreateTimer();
        resizing.Interval = TimeSpan.FromMilliseconds(500);
        resizing.IsRepeating = false;
        resizing.Tick += (_, _) => ResizeDisplay();

        revealing = DispatcherQueue.CreateTimer();
        revealing.Interval = RevealDelay;
        revealing.IsRepeating = false;
        revealing.Tick += (_, _) => Reveal(true);

        if (session.Cursor is { } shape)
        {
            cursor.Show(shape);
        }

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
        Bar.PointerExited += (_, _) => Reveal(false);
        Bar.SizeChanged += (_, _) => UpdateTitleBar();
        Bar.LayoutUpdated += (_, _) => UpdatePassthrough();
        Bar.Loaded += (_, _) => Content.XamlRoot.Changed += (_, _) => UpdateTitleBar();
    }

    // The right edge keeps room for the caption buttons, which full screen removes. Changing it moves the buttons,
    // and UpdatePassthrough follows them once layout has run.
    private void UpdateTitleBar()
    {
        if (closed || Content.XamlRoot is not { } root)
        {
            return;
        }

        // A minimized window reports a scale of 0, and an infinite width throws. The inset waits for a real scale.
        var inset = FullScreen ? 0 : AppWindow.TitleBar.RightInset / root.RasterizationScale;

        if (double.IsFinite(inset) && inset >= 0)
        {
            CaptionInset.Width = inset;
        }
    }

    // The whole bar drags the window, except the buttons, whose rectangles pass clicks through. They are measured after
    // every layout pass, since the inset, the window size, and full screen all move them without resizing them. Stale
    // rectangles left over the caption buttons would swallow their clicks. Only changes go to the window.
    private void UpdatePassthrough()
    {
        // A minimized window has nothing to click, and full screen has no title bar regions at all.
        if (closed || FullScreen || AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } || Content.XamlRoot is not { } root)
        {
            return;
        }

        var scale = root.RasterizationScale;
        var buttons = new[] { Toolbar, Actions }
            .Where(element => element.ActualWidth > 0)
            .Select(element =>
            {
                var bounds = element.TransformToVisual(null).TransformBounds(new Rect(0, 0, element.ActualWidth, element.ActualHeight));
                return new RectInt32((int)(bounds.X * scale), (int)(bounds.Y * scale), (int)Math.Ceiling(bounds.Width * scale), (int)Math.Ceiling(bounds.Height * scale));
            })
            .ToArray();

        if (buttons.SequenceEqual(passthrough))
        {
            return;
        }

        passthrough = buttons;

        // Layout runs inside the window's own size and state changes, where setting regions crashes the windowing
        // layer. They are set once that work returns, with the latest rectangles if several layouts queued up.
        if (!applying)
        {
            applying = true;
            DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
            {
                applying = false;

                if (!closed)
                {
                    InputNonClientPointerSource.GetForWindowId(AppWindow.Id).SetRegionRects(NonClientRegionKind.Passthrough, passthrough);
                }
            });
        }
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
                ScaleCursor();
                break;
            case Event.Layout:
                ScaleCursor();
                break;
        }
    }

    // The cursor image is in the Mac's points. On screen a point covers the Mac's pixels per point times the fit.
    private void ScaleCursor()
    {
        if (session.Layout is { } layout && layout.LogicalWidth > 0 && PixelWidth > 0 && PixelHeight > 0)
        {
            var fit = Video.Fit.Of(PixelWidth, PixelHeight, layout.PixelWidth, layout.PixelHeight);
            cursor.Scale(fit.Scale * layout.PixelWidth / layout.LogicalWidth);
        }
    }

    /// <summary>The session ended. The window closes without disconnecting again.</summary>
    public void Ended()
    {
        if (!ended)
        {
            ended = true;
            closed = true;
            Close();
        }
    }

    public void ToggleFullScreen()
    {
        AppWindow.SetPresenter(FullScreen ? AppWindowPresenterKind.Default : AppWindowPresenterKind.FullScreen);
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
        session.Connection.Video.Resize(PixelWidth, PixelHeight, Scale);
        ScaleCursor();
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
        var client = new SizeInt32((int)(width * scale), (int)((height + Bar.Height) * scale));
        AppWindow.ResizeClient(client);
        AppWindow.Move(new PointInt32(
            area.X + Math.Max(0, (area.Width - AppWindow.Size.Width) / 2),
            area.Y + Math.Max(0, (area.Height - AppWindow.Size.Height) / 2)));
    }

    // In full screen the screen spans both rows and the title bar becomes an overlay, hidden until revealed.
    // An exception escaping an AppWindow handler makes the windowing layer end the process, so it is logged instead.
    private void OnChanged(AppWindow sender, AppWindowChangedEventArgs args)
    {
        try
        {
            Changed(args);
        }
        catch (Exception error)
        {
            Log.Error("Cannot follow a window change", error);
        }
    }

    private void Changed(AppWindowChangedEventArgs args)
    {
        // Maximizing changes the caption buttons' size, so the reserved room follows.
        if (args.DidSizeChange && !args.DidPresenterChange)
        {
            UpdateTitleBar();
        }

        if (!args.DidPresenterChange)
        {
            return;
        }

        var full = FullScreen;

        // In full screen the bar is an overlay of buttons, not a title bar: dragging it must not move the window.
        ExtendsContentIntoTitleBar = !full;
        passthrough = [];

        if (!full)
        {
            SetTitleBar(Bar);
            AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        }

        Grid.SetRow(Screen, full ? 0 : 1);
        Grid.SetRowSpan(Screen, full ? 2 : 1);
        FullScreenIcon.Glyph = full ? "" : "";
        Bar.Background = full ? (Brush)Application.Current.Resources["AcrylicInAppFillColorDefaultBrush"] : null;
        Reveal(!full);
        UpdateTitleBar();
    }

    private void Reveal(bool visible)
    {
        revealing.Stop();

        // A menu opened from the overlay keeps it down until it closes.
        if (!visible && (!FullScreen || (Content.XamlRoot is { } root && VisualTreeHelper.GetOpenPopupsForXamlRoot(root).Count > 0)))
        {
            return;
        }

        Bar.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
    }

    // The phase names what happened for mouse and pen too: Began is a press and Ended a release.
    private void OnPointer(PointerRoutedEventArgs e, TouchPhase phase)
    {
        var point = e.GetCurrentPoint(Surface);

        // Like the RDP connection bar, the bar comes down only from the middle of the top edge and not mid-drag, so the
        // rest of the top edge stays the Mac's, for its menu bar and window title bars.
        if (FullScreen && Bar.Visibility == Visibility.Collapsed)
        {
            if (point.Position.Y <= 2 && !point.IsInContact && Math.Abs(point.Position.X - Surface.ActualWidth / 2) <= Surface.ActualWidth / 6)
            {
                if (!revealing.IsRunning)
                {
                    revealing.Start();
                }
            }
            else
            {
                revealing.Stop();
            }
        }

        if (MacPoint(point.Position) is not { } mac)
        {
            return;
        }

        if (e.Pointer.PointerDeviceType == PointerDeviceType.Touch)
        {
            session.Connection.Touch(e.Pointer.PointerId, phase, (float)mac.X, (float)mac.Y);
        }
        else
        {
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
        if (e.Pointer.PointerDeviceType == PointerDeviceType.Touch && MacPoint(e.GetCurrentPoint(Surface).Position) is { } mac)
        {
            session.Connection.Touch(e.Pointer.PointerId, TouchPhase.Cancelled, (float)mac.X, (float)mac.Y);
        }
    }

    // Windows has no system line height. Edge and Chrome scroll 100 DIPs a notch at the default 3 lines, so a line is a third of that.
    private const float LineHeight = 100f / 3;

    // One notch is 120 and scrolls as many lines as the Windows setting says, read each time so changes apply at once.
    // The distance is those lines at LineHeight, in the Mac's points. There is no acceleration, as with a wheel on a Mac.
    // Windows counts up and right as positive, the session up and left.
    private void OnWheel(object sender, PointerRoutedEventArgs e)
    {
        var properties = e.GetCurrentPoint(Surface).Properties;
        var notches = properties.MouseWheelDelta / 120f;
        var points = LineHeight * MacPointsPerDip();

        if (properties.IsHorizontalMouseWheel)
        {
            var lines = -notches * WheelLines(Native.SpiGetWheelScrollChars);
            session.Connection.Wheel(lines * points, 0, lines, 0);
        }
        else
        {
            var lines = notches * WheelLines(Native.SpiGetWheelScrollLines);
            session.Connection.Wheel(0, lines * points, 0, lines);
        }

        e.Handled = true;
    }

    // How many of the Mac's points one DIP of the window covers, undoing the monitor scale, the fit, and the Mac's own scale.
    private float MacPointsPerDip()
    {
        if (session.Layout is not { } layout || layout.PixelWidth == 0 || PixelWidth == 0 || PixelHeight == 0)
        {
            return 1;
        }

        var fit = Video.Fit.Of(PixelWidth, PixelHeight, layout.PixelWidth, layout.PixelHeight);
        return (float)(Surface.CompositionScaleX / fit.Scale * layout.LogicalWidth / layout.PixelWidth);
    }

    // Lines per notch, 3 by default. "One screen at a time" has no counterpart on the Mac, so it scrolls the default.
    private static unsafe uint WheelLines(uint setting)
    {
        uint lines = 3;
        return Native.SystemParametersInfo(setting, 0, &lines, 0) && lines != Native.WheelPageScroll ? lines : 3;
    }

    // Converts a position on the screen area to pixels of the Mac's layout, undoing the placement the renderer used.
    private Point? MacPoint(Point position)
    {
        if (session.Layout is not { } layout || layout.PixelWidth == 0 || layout.PixelHeight == 0 || PixelWidth == 0 || PixelHeight == 0)
        {
            return null;
        }

        var fit = Video.Fit.Of(PixelWidth, PixelHeight, layout.PixelWidth, layout.PixelHeight);
        var x = Math.Clamp((position.X * Surface.CompositionScaleX - fit.Left) / fit.Scale, 0, layout.PixelWidth - 1);
        var y = Math.Clamp((position.Y * Surface.CompositionScaleY - fit.Top) / fit.Scale, 0, layout.PixelHeight - 1);
        return new Point(x, y);
    }

    // The Mac's default shortcuts: Control-Up Arrow, Control-Down Arrow, F11
    private void OnMissionControl(object sender, RoutedEventArgs e) => Press(Keysyms.ControlL, Keysyms.Up);

    private void OnAppWindows(object sender, RoutedEventArgs e) => Press(Keysyms.ControlL, Keysyms.Down);

    private void OnDesktop(object sender, RoutedEventArgs e) => Press(Keysyms.F11);

    private void OnFullScreen(object sender, RoutedEventArgs e) => ToggleFullScreen();

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
