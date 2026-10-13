using Naru.Core;
using Windows.Storage;

namespace Naru.Store;

/// <summary>Where Windows key combinations such as Alt+Tab and the Windows key go, as in Remote Desktop.</summary>
internal enum WindowsKeys
{
    ThisComputer,
    Mac,
    FullScreenOnly,
}

/// <summary>
/// Settings shared by every Mac, stored in the app's local settings.
/// </summary>
/// <remarks>Sessions read them on connect and the keyboard reads them on each key, so a change takes effect from then on.</remarks>
internal sealed class Preferences
{
    private readonly ApplicationDataContainer store = ApplicationData.Current.LocalSettings;

    /// <summary>Whether to favor latency or quality when packets are lost. Latency comes first, as in Screen Sharing.</summary>
    public Priority Priority
    {
        get => Read("priority", "latency") == "quality" ? Priority.Quality : Priority.Latency;
        set => Write("priority", value == Priority.Quality ? "quality" : "latency");
    }

    /// <summary>Whether Alt sends Command and the Windows key sends Option, matching the keys on a Mac keyboard.</summary>
    public bool SwapCommandOption
    {
        get => Read("swapCommandOption", true);
        set => Write("swapCommandOption", value);
    }

    /// <summary>Whether the Mac's input source follows this computer's input language, as Sync Keyboard Language does in Screen Sharing.</summary>
    public bool SyncKeyboardLanguage
    {
        get => Read("syncKeyboardLanguage", false);
        set => Write("syncKeyboardLanguage", value);
    }

    public WindowsKeys WindowsKeys
    {
        get => Enum.TryParse<WindowsKeys>(Read("windowsKeys", nameof(WindowsKeys.FullScreenOnly)), out var value) ? value : WindowsKeys.FullScreenOnly;
        set => Write("windowsKeys", value.ToString());
    }

    /// <summary>Whether to keep the clipboard in sync with the Mac.</summary>
    public bool SharedClipboard
    {
        get => Read("sharedClipboard", true);
        set => Write("sharedClipboard", value);
    }

    // Stored values are strings and plain types so they still read back if enum names change.
    private T Read<T>(string key, T fallback) => store.Values[key] is T value ? value : fallback;

    private void Write<T>(string key, T value) => store.Values[key] = value;
}
