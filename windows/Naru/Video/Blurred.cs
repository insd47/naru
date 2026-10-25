using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Effects;
using Windows.Graphics.DirectX;

namespace Naru.Video;

/// <summary>A blurred picture as BGRA rows without padding, for the last picture kept while the Mac's display changes size.</summary>
/// <remarks>
/// <list type="bullet">
/// <item>Win2D blurs a quarter-size copy on its own device, which looks the same as blurring the whole picture and costs a sixteenth.</item>
/// <item>A 12 pixel blur at a quarter is about 48 pixels at full size, close to Screen Sharing's. Hard borders keep the edges opaque.</item>
/// </list>
/// </remarks>
internal sealed record Blurred(byte[] Pixels, int Width, int Height)
{
    private const float Scale = 0.25f;

    /// <summary>Blurs a picture given as BGRA rows without padding.</summary>
    public static Blurred Of((byte[] Pixels, int Width, int Height) source)
    {
        var device = CanvasDevice.GetSharedDevice();
        using var picture = CanvasBitmap.CreateFromBytes(device, source.Pixels, source.Width, source.Height, DirectXPixelFormat.B8G8R8A8UIntNormalized);
        using var target = new CanvasRenderTarget(device, Math.Max(1, source.Width * Scale), Math.Max(1, source.Height * Scale), 96);

        using (var session = target.CreateDrawingSession())
        {
            var small = new ScaleEffect { Source = picture, Scale = new Vector2(Scale), BorderMode = EffectBorderMode.Hard };
            session.DrawImage(new GaussianBlurEffect { Source = small, BlurAmount = 12, BorderMode = EffectBorderMode.Hard });
        }

        var size = target.SizeInPixels;
        return new Blurred(target.GetPixelBytes(), (int)size.Width, (int)size.Height);
    }
}
