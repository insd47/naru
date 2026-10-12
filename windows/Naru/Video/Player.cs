using Naru.Core;

namespace Naru.Video;

/// <summary>
/// Decodes the session's video and presents it.
/// </summary>
/// <remarks>Called on the session's event thread.</remarks>
internal sealed class Player(Action requestKeyframe) : IDisposable
{
    public void Configure(DecoderConfig config)
    {
        requestKeyframe();
    }

    public void Queue(AccessUnit unit)
    {
    }

    public void Reset()
    {
        requestKeyframe();
    }

    public void Report(MediaStats stats)
    {
    }

    public void Dispose()
    {
    }
}
