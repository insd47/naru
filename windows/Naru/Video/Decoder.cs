namespace Naru.Video;

/// <summary>Hardware video decoding through Media Foundation.</summary>
internal static class Decoder
{
    /// <summary>Whether this PC can decode the Mac's HEVC 4:4:4 in hardware. Without it, the session uses H.264.</summary>
    public static bool SupportsHevc444() => false;
}
