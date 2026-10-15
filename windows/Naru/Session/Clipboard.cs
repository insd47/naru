using Naru.Store;
using Windows.ApplicationModel.DataTransfer;
using SystemClipboard = Windows.ApplicationModel.DataTransfer.Clipboard;

namespace Naru.Session;

/// <summary>
/// Keeps clipboard text in sync between the Mac and Windows while sharing is on.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Text changed on the Mac goes into the Windows clipboard as soon as it arrives.</item>
/// <item>Text copied on Windows goes to the Mac when the session window comes to the front, and right away while it is in front.</item>
/// <item>The last text exchanged is remembered so the same content is not echoed back. Content is never logged.</item>
/// <item>Line breaks are LF on the Mac and CRLF on Windows, converted each way. Text is compared in the Mac's form.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class Clipboard : IDisposable
{
    private readonly Connection connection;
    private readonly Preferences preferences;
    private string? last;
    private bool active;

    public Clipboard(Connection connection, Preferences preferences)
    {
        this.connection = connection;
        this.preferences = preferences;
        SystemClipboard.ContentChanged += OnChanged;
        // Text already on the clipboard at connect is not sent; only text copied after connecting is.
        _ = Remember();
    }

    public void Dispose()
    {
        SystemClipboard.ContentChanged -= OnChanged;
    }

    /// <summary>The Mac's clipboard changed. If it is our own text echoed back, the Windows clipboard is left alone.</summary>
    public void OnServer(string text)
    {
        if (!preferences.SharedClipboard || text == last)
        {
            return;
        }

        last = text;
        var package = new DataPackage();
        package.SetText(text.ReplaceLineEndings("\r\n"));
        SystemClipboard.SetContent(package);
    }

    public void Activated()
    {
        active = true;
        _ = Sync();
    }

    public void Deactivated() => active = false;

    private void OnChanged(object? sender, object e)
    {
        if (active)
        {
            _ = Sync();
        }
    }

    // Sends the Windows clipboard text if it differs from the Mac's.
    private async Task Sync()
    {
        if (!preferences.SharedClipboard || await Read() is not { Length: > 0 } text || text == last)
        {
            return;
        }

        if (!connection.Clipboard(text))
        {
            Log.Info($"Not sending clipboard of {text.Length} chars: too large");
            return;
        }

        last = text;
        Log.Info($"Sent clipboard of {text.Length} chars to the Mac");
    }

    private async Task Remember() => last ??= await Read();

    // Returns the text in the Mac's form. Another app may hold the clipboard open, in which case reading fails until it lets go.
    private static async Task<string?> Read()
    {
        try
        {
            var content = SystemClipboard.GetContent();
            return content.Contains(StandardDataFormats.Text) ? (await content.GetTextAsync()).ReplaceLineEndings("\n") : null;
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
        {
            Log.Info("Cannot read the clipboard");
            return null;
        }
    }
}
