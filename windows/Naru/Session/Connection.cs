using Naru.Audio;
using Naru.Core;
using Naru.Video;

namespace Naru.Session;

/// <summary>
/// Opens one session and routes the events the core pushes to video, audio, and the UI.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Events arrive on the session's event thread. <see cref="Player"/> hands video to its own thread, and audio plays directly on this one.</item>
/// <item>Events other than video and audio go to the listener on this thread, so the UI side posts them to its own thread.</item>
/// <item>The core discards input sent after the session ends.</item>
/// </list>
/// </remarks>
internal sealed class Connection : SessionListener
{
    private readonly Action<Event> listener;
    private readonly Core.Session session;
    private Speaker? audio;
    private volatile InputSource? source;

    public Player Video { get; }

    /// <summary>The Mac's latest input source, which decides what keys type there. Written on the session's event thread.</summary>
    public InputSource? Source => source;

    public Connection(Config config, Action<Event> listener)
    {
        this.listener = listener;
        // The session starts its event thread while connecting, so video must exist first to receive events. Its
        // keyframe requests come only after a decoder config arrives, long after the session exists.
        Video = new Player(() => session?.RequestKeyframe());
        session = Core.Session.Connect(config, this);
    }

    public void Key(uint keysym, bool down) => session.Key(keysym, null, down);

    public void SelectInputSource(string id) => session.SelectInputSource(id);

    public void Pointer(byte buttons, ushort x, ushort y) => session.Pointer(buttons, x, y);

    public void Scroll(Phase phase, float dx, float dy) => session.Scroll(phase, dx, dy);

    public void Momentum(MomentumPhase phase, float dx, float dy) => session.Momentum(phase, dx, dy);

    public void Wheel(float dx, float dy, float linesX, float linesY) => session.Wheel(dx, dy, linesX, linesY);

    public void Magnify(Phase phase, float scale) => session.Magnify(phase, scale);

    public void OfferClipboard(string[] kinds) => session.OfferClipboard(kinds);

    /// <summary><c>true</c> if sent; <c>false</c> without sending if it exceeds the size limit.</summary>
    public bool SendClipboard(ClipboardItem[] items) => session.SendClipboard(items);

    public void FetchClipboard() => session.FetchClipboard();

    public void Resize(DisplayMode mode) => session.Resize(mode);

    public void Disconnect() => session.Disconnect();

    // Video and audio arrive dozens of times per second, so they are neither logged nor passed to the UI.
    public void OnEvent(Event @event)
    {
        switch (@event)
        {
            case Event.Frame frame:
                Video.Queue(frame.Unit);
                return;
            case Event.Present:
                Video.Present();
                return;
            case Event.Audio audioEvent:
                Play(audioEvent.AudioFrame);
                return;
            case Event.Stats stats:
                Video.Report(stats.StatsValue);
                return;
            case Event.Source changed:
                source = changed.SourceValue;
                break;
            case Event.Decoder decoder:
                Video.Configure(decoder.Config);
                break;
            case Event.ResetDecoder:
                Video.Reset();
                break;
            case Event.Layout layout:
                Video.Crop(layout.PixelWidth, layout.PixelHeight);
                break;
            case Event.AudioConfig audioConfig:
                StartAudio(audioConfig.Config);
                break;
        }

        Log.Info(Describe(@event));
        listener(@event);

        // The core session is left to the finalizer: input may still arrive from the UI, and the core ignores it once closed.
        if (@event is Event.Closed)
        {
            Video.Dispose();
            audio?.Dispose();
            audio = null;
        }
    }

    // If the decoder or the audio device fails, the next frame fails the same way, so audio is turned off. Video and other events keep flowing.
    private void Play(AudioFrame frame)
    {
        try
        {
            audio?.Play(frame);
        }
        catch (Exception error) when (error is InvalidOperationException or SharpGen.Runtime.SharpGenException or DllNotFoundException)
        {
            Log.Error("Audio playback failed; turning audio off", error);
            audio?.Dispose();
            audio = null;
        }
    }

    // Without audio, the session continues with video only.
    private void StartAudio(byte[] config)
    {
        audio?.Dispose();
        audio = null;

        try
        {
            audio = new Speaker(config);
        }
        catch (Exception error) when (error is InvalidOperationException or SharpGen.Runtime.SharpGenException or DllNotFoundException)
        {
            Log.Error("Cannot start audio", error);
        }
    }

    // Logs record only the event kind and size, never clipboard content or key bytes.
    private static string Describe(Event @event) => @event switch
    {
        Event.Progress progress => $"Stage: {progress.Stage}",
        Event.ServerKey key => $"Server key, {key.Der.Length} bytes",
        Event.Authenticated authenticated => $"Authenticated (server proof {authenticated.ServerProof})",
        Event.Layout layout => $"Layout: logical {layout.LogicalWidth}×{layout.LogicalHeight}, pixels {layout.PixelWidth}×{layout.PixelHeight}",
        Event.Decoder decoder => $"Decoder config: {decoder.Config.Codec}, {decoder.Config.Sps.Length} SPS, {decoder.Config.Pps.Length} PPS",
        Event.Cursor cursor => $"Cursor {cursor.Width}×{cursor.Height}",
        Event.ServerName name => $"Mac name: {name.Name}",
        Event.Source source => $"Input source: {source.SourceValue.Id} (secure input {source.SourceValue.Secure})",
        Event.ClipboardOffer offer => $"Clipboard offer: {string.Join(", ", offer.Kinds)}",
        Event.Clipboard clipboard => $"Clipboard data: {string.Join(", ", clipboard.Items.Select(item => $"{item.Kind} {item.Data.Length} bytes"))}",
        Event.ResetDecoder => "Decoder reset requested",
        Event.AudioConfig audioConfig => $"Audio config, {audioConfig.Config.Length} bytes",
        Event.Closed closed => $"Session closed: {closed.Detail ?? "normally"}",
        _ => @event.GetType().Name,
    };
}
