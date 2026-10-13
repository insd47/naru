using System.Collections.Concurrent;
using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml.Controls;
using Naru.Core;
using SharpGen.Runtime;

namespace Naru.Video;

/// <summary>
/// Decodes the session's video and presents it in the session window.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The decoder and renderer live on one dedicated thread. Calls from any thread are posted there and handled in order.</item>
/// <item>A decoder exists only while a config (VPS, SPS, PPS) exists. When it changes, a new one is created and fed from a keyframe.</item>
/// <item>A keyframe is requested when the decoder failed or input backed up. The session requests one itself for a new config and for stall recovery.</item>
/// <item>Each new decoder raises <see cref="FirstFrame"/> once it presents its first picture. The screen is black until then.</item>
/// <item>Work arriving after <see cref="Dispose"/> is ignored.</item>
/// </list>
/// </remarks>
internal sealed class Player : IDisposable
{
    // Access units waiting on the video thread before they are dropped, about 0.5 s at 60 fps.
    private const int Backlog = 30;

    // Interval between keyframe re-requests while waiting. The session also enforces a minimum interval between requests.
    private static readonly TimeSpan KeyframeRetry = TimeSpan.FromSeconds(1);

    private readonly Action requestKeyframe;
    private readonly BlockingCollection<Action> work = [];
    private readonly Thread thread;

    // Owned by the video thread
    private Graphics? graphics;
    private Renderer? renderer;
    private Decoder? decoder;
    private DecoderConfig? config;
    private uint visibleWidth;
    private uint visibleHeight;
    private bool shown;
    private bool disposed;

    // After a new decoder is created, nothing is decoded until a keyframe arrives.
    private bool awaitingKeyframe = true;
    private long requestedAt;

    private int pending;

    // Stats arrive every half second; one line per 10 s keeps the log readable for the rest of the session.
    private const int ReportsPerLine = 20;

    // Receive counters logged with every twentieth stats event
    private int frames;
    private int keyframes;
    private int dropped;
    private int rendered;
    private int reports;

    /// <summary>The first picture of a new decoder was presented. Raised on the video thread.</summary>
    public event Action? FirstFrame;

    public Player(Action requestKeyframe)
    {
        this.requestKeyframe = requestKeyframe;
        thread = new Thread(Run) { Name = "naru-video", IsBackground = true };
        thread.Start();
    }

    /// <summary>
    /// Presents in this panel, sized in physical pixels at this scale. Called on the UI thread.
    /// </summary>
    /// <remarks>The swap chain is created on the video thread and handed to the panel back on the UI thread.</remarks>
    public void Attach(SwapChainPanel panel, uint width, uint height, float scale)
    {
        var queue = panel.DispatcherQueue;

        Post(() =>
        {
            renderer?.Dispose();
            renderer = graphics is { } device ? Guard("create the renderer", () => new Renderer(device, width, height, scale)) : null;

            if (renderer is { } created)
            {
                queue.TryEnqueue(() => created.Attach(panel));
                shown = false;
                Ask();
            }
        });
    }

    public void Resize(uint width, uint height, float scale) => Post(() => Guard("resize", () => renderer?.Resize(width, height, scale)));

    /// <summary>The visible size of the Mac's screen in pixels, which the decoded picture may pad.</summary>
    public void Show(uint width, uint height) => Post(() => (visibleWidth, visibleHeight) = (width, height));

    // The server resends the same config with every keyframe. The decoder is kept for an identical config and recreated only when it changes.
    // When recreating, the following keyframe arrives shortly, so none is requested.
    public void Configure(DecoderConfig next) => Post(() =>
    {
        if (decoder == null || config is not { } current || !Same(current, next))
        {
            config = next;
            Rebuild(keyframe: false);
        }
    });

    public void Queue(AccessUnit unit)
    {
        Interlocked.Increment(ref pending);
        Post(() => Decode(unit));
    }

    /// <summary>No frame has come out for a while, so the decoder is recreated. The session requests the keyframe.</summary>
    public void Reset() => Post(() => Rebuild(keyframe: false));

    public void Report(MediaStats stats) => Post(() =>
    {
        if (++reports < ReportsPerLine)
        {
            return;
        }

        reports = 0;
        Log.Info(
            $"Received (10 s): frames {frames} (key {keyframes}, dropped {dropped}), rendered {rendered} | datagrams {stats.Datagrams}, " +
            $"authenticated {stats.Authenticated}, SSRC {stats.Ssrc:x}, sequence {stats.Highest}, candidates {stats.Candidates}, " +
            $"offers {stats.Offers}, NACK {stats.Nacked}, filled {stats.Filled}, lost {stats.Losses}");
        frames = keyframes = dropped = rendered = 0;
    });

    public void Dispose()
    {
        Post(() =>
        {
            disposed = true;
            decoder?.Dispose();
            renderer?.Dispose();
            graphics?.Dispose();
            decoder = null;
            renderer = null;
            graphics = null;
        });
        work.CompleteAdding();
    }

    private void Post(Action action)
    {
        if (!work.IsAddingCompleted)
        {
            try
            {
                work.Add(action);
            }
            catch (InvalidOperationException)
            {
                // Disposed between the check and the add
            }
        }
    }

    private void Run()
    {
        graphics = Guard("create the Direct3D device", () => new Graphics());

        foreach (var action in work.GetConsumingEnumerable())
        {
            if (!disposed)
            {
                action();
            }
        }
    }

    private void Decode(AccessUnit unit)
    {
        var waiting = Interlocked.Decrement(ref pending);
        frames += 1;

        if (unit.Keyframe)
        {
            keyframes += 1;
        }

        if (decoder is not { } current)
        {
            dropped += 1;
            return;
        }

        // Rendering fell behind the network: drop the backlog and resume from a keyframe.
        if (waiting >= Backlog && !unit.Keyframe)
        {
            if (!awaitingKeyframe)
            {
                Log.Info("Video backed up; dropping pending frames");
                awaitingKeyframe = true;
                Ask();
            }

            dropped += 1;
            return;
        }

        if (awaitingKeyframe && !unit.Keyframe)
        {
            dropped += 1;

            if (Stopwatch.GetElapsedTime(requestedAt) >= KeyframeRetry)
            {
                Ask();
            }

            return;
        }

        awaitingKeyframe = false;
        List<Picture> pictures;

        try
        {
            pictures = current.Decode(unit);
        }
        catch (Exception error) when (error is SharpGenException or InvalidOperationException or COMException)
        {
            Log.Error("Decoder error", error);
            Rebuild(keyframe: true);
            return;
        }

        // Only the newest picture is worth showing. Older ones from the same call are skipped.
        for (var index = 0; index < pictures.Count; index++)
        {
            var picture = pictures[index];

            if (index == pictures.Count - 1 && renderer is { } target)
            {
                Present(target, picture);
            }

            picture.Texture.Dispose();
            picture.Sample.Dispose();
        }
    }

    private void Present(Renderer target, Picture picture)
    {
        try
        {
            target.Render(picture, visibleWidth, visibleHeight);
        }
        catch (SharpGenException error)
        {
            Log.Error("Cannot present video", error);
            return;
        }

        rendered += 1;

        if (!shown)
        {
            shown = true;
            FirstFrame?.Invoke();
        }
    }

    private void Rebuild(bool keyframe)
    {
        decoder?.Dispose();
        decoder = null;
        awaitingKeyframe = true;
        shown = false;

        if (disposed || graphics is not { } device || config is not { } current)
        {
            return;
        }

        decoder = Guard("create the decoder", () => new Decoder(device, current));

        if (decoder != null)
        {
            Log.Info($"Decoder: {decoder.Name}");
        }

        if (keyframe)
        {
            Ask();
        }
    }

    private static bool Same(DecoderConfig a, DecoderConfig b)
    {
        static bool Equal(byte[][] x, byte[][] y) => x.Length == y.Length && x.Zip(y).All(pair => pair.First.AsSpan().SequenceEqual(pair.Second));

        return a.Codec == b.Codec && Equal(a.Vps, b.Vps) && Equal(a.Sps, b.Sps) && Equal(a.Pps, b.Pps);
    }

    private void Ask()
    {
        requestedAt = Stopwatch.GetTimestamp();
        requestKeyframe();
    }

    private static T? Guard<T>(string what, Func<T> create) where T : class
    {
        try
        {
            return create();
        }
        catch (Exception error) when (error is SharpGenException or InvalidOperationException or COMException)
        {
            Log.Error($"Cannot {what}", error);
            return null;
        }
    }

    private static void Guard(string what, Action action)
    {
        try
        {
            action();
        }
        catch (Exception error) when (error is SharpGenException or COMException)
        {
            Log.Error($"Cannot {what}", error);
        }
    }
}
