using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Windows.Graphics;

namespace Naru.Session;

/// <summary>
/// The session window's title bar, which holds the toolbar and becomes an overlay in full screen.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The whole bar drags the window, except the buttons, whose rectangles pass clicks through. The right edge keeps room for the caption buttons.</item>
/// <item>In full screen it is a bar of buttons that drags nothing. As in Screen Sharing on the Mac, it slides down when the pointer rests at the top edge for half a second, moving the screen down by its height rather than covering it, and hides when the pointer leaves. The screen moves rather than shrinks, since a new size would resize the Mac's display.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class Chrome
{
    // How long the pointer rests at the top edge in full screen before the bar slides down, as in Screen Sharing on the Mac
    private static readonly TimeSpan RevealDelay = TimeSpan.FromMilliseconds(500);

    private readonly Window window;
    private readonly Grid bar;
    private readonly FrameworkElement[] buttons;
    private readonly FrameworkElement inset;
    private readonly TranslateTransform shift = new();
    private readonly DispatcherQueueTimer revealing;
    private RectInt32[] passthrough = [];
    private bool applying;
    private bool closed;

    public Chrome(Window window, Grid bar, FrameworkElement[] buttons, FrameworkElement inset, UIElement screen)
    {
        this.window = window;
        this.bar = bar;
        this.buttons = buttons;
        this.inset = inset;
        screen.RenderTransform = shift;

        window.ExtendsContentIntoTitleBar = true;
        window.SetTitleBar(bar);
        // Tall caption buttons match the 48 px bar, so they line up with its buttons.
        window.AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;

        revealing = window.DispatcherQueue.CreateTimer();
        revealing.Interval = RevealDelay;
        revealing.IsRepeating = false;
        revealing.Tick += (_, _) => Reveal(true);

        bar.PointerExited += (_, _) => Reveal(false);
        bar.SizeChanged += (_, _) =>
        {
            Inset();
            Shift();
        };
        bar.LayoutUpdated += (_, _) => Passthrough();
        bar.Loaded += (_, _) => window.Content.XamlRoot.Changed += (_, _) => Inset();
    }

    private bool FullScreen => window.AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen;

    /// <summary>Follows a change of the window's size or presenter.</summary>
    public void Follow(AppWindowChangedEventArgs args)
    {
        if (args.DidPresenterChange)
        {
            var full = FullScreen;
            window.ExtendsContentIntoTitleBar = !full;
            passthrough = [];

            if (!full)
            {
                window.SetTitleBar(bar);
                window.AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
            }

            bar.Background = full ? (Brush)Application.Current.Resources["AcrylicInAppFillColorDefaultBrush"] : null;
            Reveal(!full);
        }

        // Maximizing changes the caption buttons' size, so the reserved room follows.
        if (args.DidSizeChange || args.DidPresenterChange)
        {
            Inset();
        }
    }

    /// <summary>The pointer moved over the screen. In full screen, resting at the top edge slides the bar down, except mid-drag.</summary>
    public void Hover(Point position, bool pressed)
    {
        if (!FullScreen || bar.Visibility == Visibility.Visible)
        {
            return;
        }

        if (position.Y <= 2 && !pressed)
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

    /// <summary>The window is closing. Layout may run once more while it goes away, when its regions can no longer be set.</summary>
    public void Close()
    {
        closed = true;
        revealing.Stop();
    }

    private void Reveal(bool visible)
    {
        revealing.Stop();

        // A menu opened from the overlay keeps it down until it closes.
        if (!visible && (!FullScreen || (window.Content.XamlRoot is { } root && VisualTreeHelper.GetOpenPopupsForXamlRoot(root).Count > 0)))
        {
            return;
        }

        bar.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
        Shift();
    }

    // In full screen the screen sits under the bar, so it moves down by the bar's height while the bar shows.
    private void Shift()
    {
        shift.Y = FullScreen && bar.Visibility == Visibility.Visible ? bar.ActualHeight : 0;
    }

    // A minimized window reports a scale of 0, and an infinite width throws, so the inset waits for a real scale.
    private void Inset()
    {
        if (closed || window.Content.XamlRoot is not { } root)
        {
            return;
        }

        var width = FullScreen ? 0 : window.AppWindow.TitleBar.RightInset / root.RasterizationScale;

        if (double.IsFinite(width) && width >= 0)
        {
            inset.Width = width;
        }
    }

    // Measured after every layout pass, since the inset, the window size, and full screen all move the buttons without
    // resizing them. Stale rectangles left over the caption buttons would swallow their clicks.
    private void Passthrough()
    {
        if (closed || FullScreen || window.AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } || window.Content.XamlRoot is not { } root)
        {
            return;
        }

        var scale = root.RasterizationScale;
        var rects = buttons
            .Where(element => element.ActualWidth > 0)
            .Select(element =>
            {
                var bounds = element.TransformToVisual(null).TransformBounds(new Rect(0, 0, element.ActualWidth, element.ActualHeight));
                return new RectInt32((int)(bounds.X * scale), (int)(bounds.Y * scale), (int)Math.Ceiling(bounds.Width * scale), (int)Math.Ceiling(bounds.Height * scale));
            })
            .ToArray();

        if (rects.SequenceEqual(passthrough))
        {
            return;
        }

        passthrough = rects;

        // Layout runs inside the window's own size and state changes, where setting regions crashes the windowing
        // layer. They are set once that work returns, with the latest rectangles if several layouts queued up.
        if (applying)
        {
            return;
        }

        applying = true;
        window.DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            applying = false;

            if (!closed)
            {
                InputNonClientPointerSource.GetForWindowId(window.AppWindow.Id).SetRegionRects(NonClientRegionKind.Passthrough, passthrough);
            }
        });
    }
}
