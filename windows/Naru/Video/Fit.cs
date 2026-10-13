namespace Naru.Video;

/// <summary>
/// Where the Mac's screen sits in the screen area: centered, shrunk to fit, and never enlarged.
/// </summary>
/// <remarks>
/// A virtual display follows the window, but the Mac may round its size, so the two can differ by a few pixels.
/// Resampling for that difference blurs text, so a picture that fits is drawn at one Mac pixel per screen pixel, as
/// Screen Sharing shows a smaller screen at full size. Both the renderer and pointer input use this, so clicks land
/// where the picture is.
/// </remarks>
internal readonly record struct Fit(double Scale, double Left, double Top)
{
    /// <summary>The placement of a <paramref name="sourceWidth"/> × <paramref name="sourceHeight"/> picture in an area, all in pixels.</summary>
    public static Fit Of(double areaWidth, double areaHeight, double sourceWidth, double sourceHeight)
    {
        var scale = Math.Min(1, Math.Min(areaWidth / sourceWidth, areaHeight / sourceHeight));
        return new Fit(scale, Math.Floor((areaWidth - sourceWidth * scale) / 2), Math.Floor((areaHeight - sourceHeight * scale) / 2));
    }
}
