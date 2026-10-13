using System.Runtime.InteropServices.WindowsRuntime;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using Naru.Core;
using Windows.Foundation;

namespace Naru.Session;

/// <summary>
/// Draws the Mac's cursor where finger touches moved it.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Fingers drive the session's trackpad emulation, which moves the cursor relatively and reports where it is. Touch
/// has no system pointer, so the image is drawn there, as the iPad and Android apps do.</item>
/// <item>A mouse or pen has the system pointer, so using one hides the drawn cursor.</item>
/// <item>The image is the Mac's logical size, stretched to the screen with filtering.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class TouchCursor(Image image)
{
    private Event.Cursor? shape;

    public void Show(Event.Cursor next)
    {
        shape = next;

        if (next.Width == 0 || next.Height == 0)
        {
            image.Source = null;
            return;
        }

        // WriteableBitmap takes premultiplied BGRA, and the Mac sends straight RGBA.
        var bitmap = new WriteableBitmap(next.Width, next.Height);
        var pixels = new byte[next.Rgba.Length];

        for (var at = 0; at < pixels.Length; at += 4)
        {
            var alpha = next.Rgba[at + 3];
            pixels[at] = (byte)(next.Rgba[at + 2] * alpha / 255);
            pixels[at + 1] = (byte)(next.Rgba[at + 1] * alpha / 255);
            pixels[at + 2] = (byte)(next.Rgba[at] * alpha / 255);
            pixels[at + 3] = alpha;
        }

        using (var stream = bitmap.PixelBuffer.AsStream())
        {
            stream.Write(pixels);
        }

        bitmap.Invalidate();
        image.Source = bitmap;
    }

    /// <summary>Places the hotspot at a point of the screen area, sized by the DIPs one of the Mac's points covers.</summary>
    public void Place(Point at, double dipsPerPoint)
    {
        if (shape is not { Width: > 0, Height: > 0 } current)
        {
            return;
        }

        image.Width = current.Width * dipsPerPoint;
        image.Height = current.Height * dipsPerPoint;
        Canvas.SetLeft(image, at.X - current.HotspotX * dipsPerPoint);
        Canvas.SetTop(image, at.Y - current.HotspotY * dipsPerPoint);
        image.Visibility = Visibility.Visible;
    }

    public void Hide() => image.Visibility = Visibility.Collapsed;
}
