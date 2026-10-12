using Naru.Core;
using Naru.Video;

namespace Naru.Session;

/// <summary>
/// Opens one session and routes the events the core pushes to video and the UI.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Events arrive on the session's event thread. <see cref="Player"/> hands video to its own thread.</item>
/// <item>Events other than video and audio go to the listener on this thread, so the UI side posts them to its own thread.</item>
/// <item>The core discards input sent after the session ends.</item>
/// </list>
/// </remarks>
internal sealed class Connection : SessionListener
{
    private readonly Action<Event> listener;
    private readonly Core.Session session;

    public Player Video { get; }

    public Connection(Config config, Action<Event> listener)
    {
        this.listener = listener;
        // The session starts its event thread while connecting, so video must exist first to receive events. Its
        // keyframe requests come only after a decoder config arrives, long after the session exists.
        Video = new Player(() => session?.RequestKeyframe());
        session = Core.Session.Connect(config, this);
    }

    public void Key(uint keysym, bool down) => session.Key(keysym, down);

    public void Pointer(byte buttons, ushort x, ushort y) => session.Pointer(buttons, x, y);

    public void Scroll(Phase phase, float dx, float dy) => session.Scroll(phase, dx, dy);

    public void Wheel(float dx, float dy) => session.Wheel(dx, dy);

    public void Touch(uint id, TouchPhase phase, float x, float y) => session.Touch(id, phase, x, y);

    /// <summary><c>true</c> if sent; <c>false</c> without sending if it exceeds the size limit.</summary>
    public bool Clipboard(string text) => session.Clipboard(text);

    public void Resize(DisplayMode mode) => session.Resize(mode);

    public void Disconnect() => session.Disconnect();

    // Video and audio arrive dozens of times per second, so they are neither logged nor passed to the UI.
    // Audio is not played yet: Media Foundation has no AAC-ELD decoder.
    public void OnEvent(Event @event)
    {
        switch (@event)
        {
            case Event.Frame frame:
                Video.Queue(frame.Unit);
                break;
            case Event.Audio or Event.AudioConfig:
                break;
            case Event.Stats stats:
                Video.Report(stats.StatsValue);
                break;
            default:
                Announce(@event);
                break;
        }
    }

    private void Announce(Event @event)
    {
        switch (@event)
        {
            case Event.Decoder decoder:
                Video.Configure(decoder.Config);
                break;
            case Event.ResetDecoder:
                Video.Reset();
                break;
            case Event.Layout layout:
                Video.Show(layout.PixelWidth, layout.PixelHeight);
                break;
        }

        // The touch cursor position arrives on every finger movement, so it is not logged.
        if (@event is not Event.Pointer)
        {
            Log.Info(Describe(@event));
        }

        listener(@event);

        // The core session is left to the finalizer: input may still arrive from the UI, and the core ignores it once closed.
        if (@event is Event.Closed)
        {
            Video.Dispose();
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
        Event.Clipboard clipboard => $"Clipboard, {clipboard.Text.Length} chars",
        Event.ResetDecoder => "Decoder reset requested",
        Event.Closed closed => $"Session closed: {closed.Detail ?? "normally"}",
        _ => @event.GetType().Name,
    };
}
