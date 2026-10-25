using Microsoft.UI.Dispatching;
using Naru.Core;
using Naru.Store;

namespace Naru.Session;

/// <summary>Username and password for connecting. On successful authentication, the password is saved if <see cref="Remember"/> is set.</summary>
internal sealed record Credentials(string Username, string Password, bool Remember);

/// <summary>
/// One open session and what the app keeps for it.
/// </summary>
/// <remarks>Credentials are held only until authentication. <see cref="Display"/> is absent on the Mac's real display.</remarks>
internal sealed class Running
{
    public required Host Host { get; set; }
    public required Credentials? Credentials { get; set; }
    public required DisplayMode? Display { get; set; }
    public required Connection Connection { get; init; }

    public Stage Stage { get; set; } = Stage.Warmup;

    /// <summary>Authentication succeeded. The video may still be getting ready.</summary>
    public bool Authenticated { get; set; }

    public SessionWindow? Window { get; set; }

    // Only the last layout and cursor are kept, for a session window that opens after they arrive.
    public Event.Layout? Layout { get; set; }
    public Event.Cursor? Cursor { get; set; }
}

/// <summary>
/// The open sessions, at most one per saved Mac.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Lives on the UI thread. Session events are posted here from each session's event thread.</item>
/// <item>Saving connection results (the Mac list, password, server key) happens here, without a window.</item>
/// <item>A session window opens once authentication succeeds, and closing it disconnects.</item>
/// </list>
/// </remarks>
internal sealed class Sessions(Hosts hosts)
{
    private readonly DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
    private readonly Dictionary<string, Running> running = [];

    /// <summary>A session started, changed stage, authenticated, or ended.</summary>
    public event Action<Host>? Changed;

    /// <summary>A session ended with a failure.</summary>
    public event Action<Host, Failure>? Failed;

    public Running? Find(string hostId) => running.GetValueOrDefault(hostId);

    public Running? FindAddress(string? address, int port) =>
        running.Values.FirstOrDefault(session => session.Host.Address == address && session.Host.Port == port);

    /// <summary>Opens a session with this Mac. Does nothing if one is already open.</summary>
    public void Open(Host host, Credentials credentials, DisplayMode? display)
    {
        if (running.ContainsKey(host.Id))
        {
            return;
        }

        var config = new Config(
            Host: host.Address,
            Port: (ushort)host.Port,
            Username: credentials.Username,
            Password: credentials.Password,
            KnownKey: ServerKeys.Find(host),
            Display: display,
            Audio: true,
            Codec: Video.Decoder.SupportsHevc444() ? VideoCodec.Hevc : VideoCodec.H264);

        Running? session = null;
        var connection = new Connection(config, @event => queue.TryEnqueue(() => Handle(session!, @event)));
        session = new Running { Host = host, Credentials = credentials, Display = display, Connection = connection };
        running[host.Id] = session;
        Changed?.Invoke(host);
    }

    public void Close(string hostId)
    {
        Find(hostId)?.Connection.Disconnect();
    }

    public void CloseAll()
    {
        foreach (var session in running.Values)
        {
            session.Connection.Disconnect();
        }
    }

    /// <summary>
    /// Resizes the virtual display. Does nothing on the Mac's real display or when the size is unchanged, and returns false then.
    /// </summary>
    public bool Resize(Running session, DisplayMode mode)
    {
        if (session.Display is not { } current || current == mode)
        {
            return false;
        }

        Log.Info($"Virtual display: {mode.Width}×{mode.Height} at {mode.Scale}×");
        session.Display = mode;
        session.Connection.Resize(mode);
        return true;
    }

    private void Handle(Running session, Event @event)
    {
        switch (@event)
        {
            case Event.Progress progress:
                session.Stage = progress.Stage;
                Changed?.Invoke(session.Host);
                break;
            case Event.ServerKey key:
                ServerKeys.Remember(session.Host, key.Der);
                break;
            // A session that gets no video reconnects and signs in again, and keeps the window it has.
            case Event.Authenticated:
                Remember(session);
                session.Authenticated = true;
                session.Window ??= new SessionWindow(this, session);
                session.Window.Activate();
                Changed?.Invoke(session.Host);
                break;
            case Event.ServerName name:
                Rename(session, name.Name);
                break;
            case Event.Layout layout:
                session.Layout = layout;
                break;
            case Event.Cursor cursor:
                session.Cursor = cursor;
                break;
            case Event.Closed closed:
                End(session, closed);
                return;
        }

        session.Window?.Handle(@event);
    }

    // Saves the authenticated Mac to the list, and the password if requested. The password is not held after this.
    private void Remember(Running session)
    {
        if (session.Credentials is not { } credentials)
        {
            return;
        }

        session.Credentials = null;
        session.Host = Latest(session) with { Username = credentials.Username, LastConnected = DateTimeOffset.Now };
        hosts.Save(session.Host);

        if (credentials.Remember)
        {
            Keychain.Save(session.Host.Id, credentials.Username, credentials.Password);
        }
    }

    // A Mac added without a name gets the computer name it reports. A name given by the user is kept.
    private void Rename(Running session, string name)
    {
        var host = Latest(session);

        if (name.Length == 0 || host.Name.Length > 0)
        {
            return;
        }

        session.Host = host with { Name = name };
        hosts.Save(session.Host);
        session.Window?.Rename(session.Host.Title);
        Changed?.Invoke(session.Host);
    }

    // Saves apply to the store's latest entry so edits made in the list while connecting are not overwritten.
    private Host Latest(Running session) => hosts.Find(session.Host.Id) ?? session.Host;

    // A wrong password deletes the saved one. The failure is reported before the session window closes, so the app is not left without a window.
    private void End(Running session, Event.Closed closed)
    {
        running.Remove(session.Host.Id);

        if (closed.Failure is Failure.Rejected)
        {
            Keychain.Remove(session.Host.Id);
        }

        Changed?.Invoke(session.Host);

        if (closed.Failure is { } failure)
        {
            Failed?.Invoke(session.Host, failure);
        }

        session.Window?.Ended();
    }
}
