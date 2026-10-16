using Vortice.Direct3D11;
using Vortice.DXGI;
using Vortice.Mathematics;

namespace Naru.Video;

/// <summary>
/// The Mac's screen put together from HEVC bands, each decoded as a picture of its own and stacked from the top.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The Mac sends only bands that changed, so each band keeps its last picture until a new one replaces it.</item>
/// <item>Starts black, in case a band has not been sent yet when the canvas is first shown.</item>
/// <item>Call only on the video thread.</item>
/// </list>
/// </remarks>
internal sealed class Canvas : IDisposable
{
    private readonly ID3D11DeviceContext context;

    public ID3D11Texture2D Texture { get; }
    public (uint Width, uint Height) Band { get; }
    public uint Rows { get; }
    public Colors Colors { get; }

    public Canvas(Graphics graphics, (uint Width, uint Height) band, uint rows, Colors colors)
    {
        context = graphics.Context;
        Band = band;
        Rows = rows;
        Colors = colors;

        // AYUV keeps V, U, Y, and alpha in that byte order.
        var black = new byte[band.Width * band.Height * rows * 4];
        var y = colors.StudioRange ? (byte)16 : (byte)0;

        for (var offset = 0; offset < black.Length; offset += 4)
        {
            (black[offset], black[offset + 1], black[offset + 2], black[offset + 3]) = (128, 128, y, 255);
        }

        var description = new Texture2DDescription
        {
            Width = band.Width,
            Height = band.Height * rows,
            MipLevels = 1,
            ArraySize = 1,
            Format = Format.AYUV,
            SampleDescription = new SampleDescription(1, 0),
            Usage = ResourceUsage.Default,
            // The video processor refuses an AYUV texture bound for shaders but takes one bound like the decoder's output.
            BindFlags = BindFlags.Decoder,
        };

        unsafe
        {
            fixed (byte* data = black)
            {
                Texture = graphics.Device.CreateTexture2D(description, [new SubresourceData(data, band.Width * 4)]);
            }
        }
    }

    /// <summary>Copies a decoded band into its place.</summary>
    public void Place(Picture picture)
    {
        var region = new Box(0, 0, 0, (int)Band.Width, (int)Band.Height, 1);
        context.CopySubresourceRegion(Texture, 0, 0, picture.Band * Band.Height, 0, picture.Texture, picture.Slice, region);
    }

    public void Dispose() => Texture.Dispose();
}
