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
/// <item>A keyframe is requested when the decoder failed or input backed up. The session repeats the request until one arrives, and a new config comes with its keyframe.</item>
/// <item>Each new decoder raises <see cref="FirstFrame"/> once it presents its first picture. The screen is black until then.</item>
/// <item>HEVC bands go into a canvas as they decode, and the canvas is presented when the session says an instant is complete.</item>
/// <item>Work arriving after <see cref="Dispose"/> is ignored.</item>
/// </list>
/// </remarks>
internal sealed class Player : IDisposable
{
    // How long an access unit may wait on the video thread before the waiting ones are dropped. Measured in time, since
    // the count per second depends on the frame rate and the number of bands. Decoding catches up far faster than real
    // time, while a drop costs a keyframe, so this tolerates passing hiccups such as a new decoder.
    private static readonly TimeSpan Backlog = TimeSpan.FromSeconds(0.5);

    private readonly Action requestKeyframe;
    private readonly BlockingCollection<Action> work = [];
    private readonly Thread thread;

    // Owned by the video thread
    private Graphics? graphics;
    private Renderer? renderer;
    private Decoder? decoder;
    private Canvas? canvas;
    private DecoderConfig? config;
    private uint visibleWidth;
    private uint visibleHeight;
    private bool shown;
    private bool disposed;

    // After a new decoder is created, nothing is decoded until a keyframe arrives.
    private bool awaitingKeyframe = true;

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
                requestKeyframe();
            }
        });
    }

    public void Resize(uint width, uint height, float scale) => Post(() => Guard("resize", () => renderer?.Resize(width, height, scale)));

    /// <summary>The visible size of the Mac's screen in pixels, which the decoded picture may pad.</summary>
    public void Crop(uint width, uint height) => Post(() => (visibleWidth, visibleHeight) = (width, height));

    public void Configure(DecoderConfig next) => Post(() =>
    {
        config = next;
        Rebuild(keyframe: false);
    });

    public void Queue(AccessUnit unit)
    {
        var queued = Stopwatch.GetTimestamp();
        Post(() => Decode(unit, queued));
    }

    /// <summary>Every band of one instant has been queued, so the canvas they went into is shown.</summary>
    public void Present() => Post(() =>
    {
        if (canvas is { } stacked && renderer is { } target && decoder != null && !awaitingKeyframe)
        {
            Show(() => target.Render(stacked.Texture, 0, stacked.StudioRange, visibleWidth, visibleHeight));
        }
    });

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
            $"offers {stats.Offers}, missing {stats.Missing}, filled {stats.Filled}, lost {stats.Losses}");
        frames = keyframes = dropped = rendered = 0;
    });

    public void Dispose()
    {
        Post(() =>
        {
            disposed = true;
            decoder?.Dispose();
            canvas?.Dispose();
            renderer?.Dispose();
            graphics?.Dispose();
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

    private void Decode(AccessUnit unit, long queued)
    {
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
        if (Stopwatch.GetElapsedTime(queued) >= Backlog && !unit.Keyframe)
        {
            if (!awaitingKeyframe)
            {
                Log.Info("Video backed up; dropping pending frames");
                awaitingKeyframe = true;
                requestKeyframe();
            }

            dropped += 1;
            return;
        }

        if (awaitingKeyframe && !unit.Keyframe)
        {
            dropped += 1;
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

            // A picture predicted from a lost one may fail without anything being wrong with the decoder.
            if (unit.Intact)
            {
                Rebuild(keyframe: true);
            }

            return;
        }

        // Bands go into the canvas, shown later as a whole. Otherwise only the newest picture is worth showing.
        for (var index = 0; index < pictures.Count; index++)
        {
            var picture = pictures[index];

            if (current.Band is { } band && config is { } stream)
            {
                Guard("place a band", () => Stack(band, stream.Bands, picture));
            }
            else if (index == pictures.Count - 1 && renderer is { } target)
            {
                Show(() => target.Render(picture.Texture, picture.Slice, picture.StudioRange, visibleWidth, visibleHeight));
            }

            picture.Texture.Dispose();
            picture.Sample.Dispose();
        }
    }

    // The canvas is replaced when the band size or count changes. It outlives a decoder reset, since the Mac may resend
    // only the top band after one.
    private void Stack((uint Width, uint Height) band, uint rows, Picture picture)
    {
        if (canvas is not { } existing || existing.Band != band || existing.Rows != rows)
        {
            renderer?.Forget();
            canvas?.Dispose();
            canvas = null;
            canvas = new Canvas(graphics!, band, rows, picture.StudioRange);
        }

        canvas.Place(picture);
    }

    private void Show(Action render)
    {
        try
        {
            render();
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
        renderer?.Forget();
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
            requestKeyframe();
        }
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
        catch (Exception error) when (error is SharpGenException or InvalidOperationException or COMException)
        {
            Log.Error($"Cannot {what}", error);
        }
    }
}
