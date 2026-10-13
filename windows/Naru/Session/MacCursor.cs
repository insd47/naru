using System.Runtime.InteropServices;
using Microsoft.UI.Input;
using Naru.Core;

namespace Naru.Session;

/// <summary>
/// Shows the Mac's cursor shape over the screen area.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>WinUI sets the cursor itself while it handles the pointer, and InputCustomCursor is not implemented. So the
/// image becomes a Win32 cursor, which WinUI's interop wraps in an InputCursor for the screen element.</item>
/// <item>The image arrives at the Mac's logical size and is scaled to the screen with filtering, as Apple's client does.</item>
/// <item>An empty image hides the cursor. Before the first image, the system arrow shows.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed unsafe class MacCursor(ScreenGrid screen) : IDisposable
{
    private static readonly Guid StaticsInterop = new("ac6f5065-90c4-46ce-beb7-05e138e54117");

    private Event.Cursor? image;
    private double scale;

    // The Win32 cursor behind the screen's current InputCursor. WinUI keeps using its handle, so it lives until replaced.
    private nint handle;

    /// <summary>A new shape from the Mac.</summary>
    public void Show(Event.Cursor next)
    {
        image = next;
        Rebuild();
    }

    /// <summary>Physical pixels per Mac logical point, which changes with the monitor, the fit, and the Mac's scale.</summary>
    public void Scale(double next)
    {
        if (Math.Abs(next - scale) > 0.001)
        {
            scale = next;
            Rebuild();
        }
    }

    public void Dispose()
    {
        screen.Cursor = null;
        Release(handle);
        handle = 0;
    }

    private void Rebuild()
    {
        if (image is not { } shape || scale <= 0)
        {
            return;
        }

        // A hidden cursor is a fully transparent one, since the screen's cursor cannot be "none".
        var next = shape.Width == 0 || shape.Height == 0 ? Native.CreateCursor(new byte[4], 1, 1, 0, 0) : Create(shape, scale);

        if (next == 0 || Wrap(next) is not { } cursor)
        {
            Log.Info($"Cannot make a cursor of {shape.Width}×{shape.Height}");
            Release(next);
            return;
        }

        var previous = screen.Cursor;
        screen.Cursor = cursor;
        previous?.Dispose();
        Release(handle);
        handle = next;
    }

    // IInputCursorStaticsInterop::CreateFromHCursor, the first method after IInspectable's six
    private static InputCursor? Wrap(nint cursor)
    {
        using var factory = WinRT.ActivationFactory.Get("Microsoft.UI.Input.InputCursor");

        if (Marshal.QueryInterface(factory.ThisPtr, in StaticsInterop, out var statics) != 0)
        {
            return null;
        }

        try
        {
            nint result;
            var create = (delegate* unmanaged[Stdcall]<nint, nint, nint*, int>)(*(void***)statics)[6];

            if (create(statics, cursor, &result) != 0 || result == 0)
            {
                return null;
            }

            try
            {
                return InputCursor.FromAbi(result);
            }
            finally
            {
                Marshal.Release(result);
            }
        }
        finally
        {
            Marshal.Release(statics);
        }
    }

    private static void Release(nint cursor)
    {
        if (cursor != 0)
        {
            Native.DestroyIcon(cursor);
        }
    }

    // Scales the image with bilinear filtering on premultiplied colors, so edges do not pick up dark fringes, and makes a
    // cursor from it. Windows cursors take straight alpha.
    private static nint Create(Event.Cursor shape, double scale)
    {
        int sourceWidth = shape.Width, sourceHeight = shape.Height;
        var width = Math.Max(1, (int)Math.Round(sourceWidth * scale));
        var height = Math.Max(1, (int)Math.Round(sourceHeight * scale));
        var source = shape.Rgba;
        var pixels = new byte[width * height * 4];

        for (var y = 0; y < height; y++)
        {
            var sy = Math.Clamp((y + 0.5) / scale - 0.5, 0, sourceHeight - 1);
            var y0 = (int)sy;
            var y1 = Math.Min(y0 + 1, sourceHeight - 1);
            var fy = sy - y0;

            for (var x = 0; x < width; x++)
            {
                var sx = Math.Clamp((x + 0.5) / scale - 0.5, 0, sourceWidth - 1);
                var x0 = (int)sx;
                var x1 = Math.Min(x0 + 1, sourceWidth - 1);
                var fx = sx - x0;

                double r = 0, g = 0, b = 0, a = 0;

                void Sample(int px, int py, double weight)
                {
                    var at = (py * sourceWidth + px) * 4;
                    var alpha = source[at + 3] / 255.0 * weight;
                    r += source[at] * alpha;
                    g += source[at + 1] * alpha;
                    b += source[at + 2] * alpha;
                    a += alpha;
                }

                Sample(x0, y0, (1 - fx) * (1 - fy));
                Sample(x1, y0, fx * (1 - fy));
                Sample(x0, y1, (1 - fx) * fy);
                Sample(x1, y1, fx * fy);

                var to = (y * width + x) * 4;

                if (a > 0)
                {
                    pixels[to] = (byte)Math.Round(b / a);
                    pixels[to + 1] = (byte)Math.Round(g / a);
                    pixels[to + 2] = (byte)Math.Round(r / a);
                    pixels[to + 3] = (byte)Math.Round(a * 255);
                }
            }
        }

        var hotspotX = (uint)Math.Clamp((int)Math.Round(shape.HotspotX * scale), 0, width - 1);
        var hotspotY = (uint)Math.Clamp((int)Math.Round(shape.HotspotY * scale), 0, height - 1);
        return Native.CreateCursor(pixels, width, height, hotspotX, hotspotY);
    }
}
