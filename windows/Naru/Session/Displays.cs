using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Naru.Core;
using Windows.Graphics;

namespace Naru.Session;

/// <summary>
/// Chooses virtual display modes for session windows.
/// </summary>
/// <remarks>
/// The Mac's logical size matches the window's size in Windows' device-independent pixels, so the Mac looks as large as
/// Windows does. macOS only renders at whole scales, so a 100% monitor gets 1× with pixels matching exactly, and any
/// larger scale gets 2× drawn down to fit, which stays sharp.
/// </remarks>
internal static partial class Displays
{
    /// <summary>The window's monitor scale, 1.0 at 100%.</summary>
    public static double Scale(AppWindow window) => GetDpiForWindow(Microsoft.UI.Win32Interop.GetWindowFromWindowId(window.Id)) / 96.0;

    /// <summary>The virtual display for a screen area of this many device-independent pixels.</summary>
    public static DisplayMode Mode(double width, double height, double scale) =>
        new((uint)Math.Max(1, Math.Round(width)), (uint)Math.Max(1, Math.Round(height)), scale > 1 ? 2.0 : 1.0);

    /// <summary>Size in device-independent pixels of the screen area of a new session window: most of the work area, below the bar.</summary>
    public static (int Width, int Height) Default(RectInt32 workArea, double scale) =>
        ((int)(workArea.Width * 0.8 / scale), (int)(workArea.Height * 0.8 / scale - SessionWindow.BarHeight));

    /// <summary>The virtual display for a new session window on the monitor showing this window.</summary>
    public static DisplayMode Initial(AppWindow window)
    {
        var area = DisplayArea.GetFromWindowId(window.Id, DisplayAreaFallback.Nearest).WorkArea;
        var scale = Scale(window);
        var (width, height) = Default(area, scale);
        return Mode(width, height, scale);
    }

    [LibraryImport("user32.dll")]
    private static partial uint GetDpiForWindow(nint window);
}
