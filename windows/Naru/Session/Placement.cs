using Naru.Core;
using Naru.Video;
using Windows.Foundation;

namespace Naru.Session;

/// <summary>
/// Where the Mac's screen sits in the screen area, as the renderer placed it.
/// </summary>
/// <remarks>Pointer input, wheel distances, and both cursors come from this, so they agree with the picture.</remarks>
internal readonly record struct Placement(Event.Layout Layout, Fit Fit, double Scale)
{
    /// <summary>The placement in a screen area of this many physical pixels at this monitor scale, once a layout exists.</summary>
    public static Placement? Of(Event.Layout? layout, uint width, uint height, double scale)
    {
        if (layout is not { PixelWidth: > 0, PixelHeight: > 0, LogicalWidth: > 0 } || width == 0 || height == 0)
        {
            return null;
        }

        return new Placement(layout, Fit.Of(width, height, layout.PixelWidth, layout.PixelHeight), scale);
    }

    /// <summary>Physical pixels one of the Mac's points covers on screen</summary>
    public double PixelsPerPoint => Fit.Scale * Layout.PixelWidth / Layout.LogicalWidth;

    /// <summary>The Mac's points one DIP of the screen area covers</summary>
    public double PointsPerDip => Scale / PixelsPerPoint;

    /// <summary>The pixel of the Mac's layout under a point of the screen area, in DIPs</summary>
    public Point Mac(Point screen) => new(
        Math.Clamp((screen.X * Scale - Fit.Left) / Fit.Scale, 0, Layout.PixelWidth - 1),
        Math.Clamp((screen.Y * Scale - Fit.Top) / Fit.Scale, 0, Layout.PixelHeight - 1));

    /// <summary>The point of the screen area, in DIPs, over a pixel of the Mac's layout</summary>
    public Point Screen(Point mac) => new((Fit.Left + mac.X * Fit.Scale) / Scale, (Fit.Top + mac.Y * Fit.Scale) / Scale);
}
