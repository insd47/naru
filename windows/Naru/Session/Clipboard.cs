using Naru.Core;
using Naru.Store;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;
using SystemClipboard = Windows.ApplicationModel.DataTransfer.Clipboard;

namespace Naru.Session;

/// <summary>
/// Shares text and images between the Mac and Windows while sharing is on, in two steps as Apple's client does: each
/// side offers the kinds it holds, and the data moves only when it is pasted.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The Mac's content goes on the Windows clipboard as delayed formats, which fetch the data from the Mac when an app pastes it.</item>
/// <item>Content copied on Windows is offered to the Mac when the session window comes to the front, and right away while it is in front. Its data is read and sent when the Mac asks for it.</item>
/// <item>Content this class put on the clipboard is never offered back.</item>
/// <item>Line breaks are LF on the Mac and CRLF on Windows, converted each way. Images go to the Mac as PNG.</item>
/// <item>Content is never logged, only kinds and sizes. Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class Clipboard : IDisposable
{
    private const string Text = "public.utf8-plain-text";
    private const string Png = "public.png";
    // Image kinds the Mac sends that Windows can decode
    private static readonly string[] Images = [Png, "public.tiff", "public.jpeg"];
    // How long a paste waits for the Mac's data. A large image over a VPN can take seconds.
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(30);

    private readonly Connection connection;
    private readonly Preferences preferences;
    private bool active;
    // Whether the Windows clipboard changed since it was last offered, and changes this class made itself
    private bool changed;
    private int own;
    // Kinds offered to the Mac, whose data it may ask for
    private List<string> offered = [];
    // The Mac's data for its latest offer, once fetched
    private TaskCompletionSource<List<ClipboardItem>>? fetch;

    public Clipboard(Connection connection, Preferences preferences)
    {
        this.connection = connection;
        this.preferences = preferences;
        // Content already on the clipboard at connect is not offered; only content copied after connecting is.
        SystemClipboard.ContentChanged += OnChanged;
    }

    public void Dispose()
    {
        SystemClipboard.ContentChanged -= OnChanged;
        fetch?.TrySetCanceled();
    }

    /// <summary>The Mac's clipboard changed to content of these kinds.</summary>
    public void OnOffer(IList<string> kinds)
    {
        if (!preferences.SharedClipboard)
        {
            return;
        }

        fetch?.TrySetCanceled();
        fetch = null;

        var package = new DataPackage();

        if (kinds.Contains(Text))
        {
            package.SetDataProvider(StandardDataFormats.Text, request => _ = Provide(request, items =>
                Task.FromResult<object>(Encoding.UTF8.GetString(Find(items, Text) ?? []).ReplaceLineEndings("\r\n"))));
        }

        if (Images.FirstOrDefault(kinds.Contains) is { } image)
        {
            package.SetDataProvider(StandardDataFormats.Bitmap, request => _ = Provide(request, async items =>
                RandomAccessStreamReference.CreateFromStream(await Stream(Find(items, image) ?? []))));
        }

        // Setting the content raises ContentChanged once, which is not offered back.
        own++;
        SystemClipboard.SetContent(package);
    }

    /// <summary>The Mac's clipboard data arrived.</summary>
    public void OnServer(IList<ClipboardItem> items)
    {
        Log.Info($"Clipboard data from the Mac: {Describe(items)}");
        fetch?.TrySetResult([.. items]);
    }

    /// <summary>The Mac wants the data of what was offered, as when it is pasted there.</summary>
    public async void OnRequested()
    {
        var items = await Read(offered);

        if (!connection.SendClipboard([.. items]))
        {
            Log.Info($"Not sending clipboard of {items.Sum(item => item.Data.Length)} bytes: too large");
            return;
        }

        Log.Info($"Sent clipboard data to the Mac: {Describe(items)}");
    }

    public void Activated()
    {
        active = true;
        Offer();
    }

    public void Deactivated() => active = false;

    private void OnChanged(object? sender, object e)
    {
        if (own > 0)
        {
            own--;
            return;
        }

        changed = true;

        if (active)
        {
            Offer();
        }
    }

    // Offers the Windows clipboard's kinds if it changed and is not the Mac's own content.
    private void Offer()
    {
        if (!preferences.SharedClipboard || !changed)
        {
            return;
        }

        changed = false;
        var kinds = Kinds();

        if (kinds.Count == 0)
        {
            return;
        }

        offered = kinds;
        connection.OfferClipboard([.. kinds]);
        Log.Info($"Offered the clipboard to the Mac: {string.Join(", ", kinds)}");
    }

    // Answers a paste with the Mac's data, fetched once for all formats of an offer.
    private async Task Provide(DataProviderRequest request, Func<List<ClipboardItem>, Task<object>> convert)
    {
        var deferral = request.GetDeferral();

        try
        {
            if (fetch is null)
            {
                fetch = new TaskCompletionSource<List<ClipboardItem>>();
                connection.FetchClipboard();
            }

            var items = await fetch.Task.WaitAsync(Timeout);
            request.SetData(await convert(items));
        }
        catch (Exception error) when (error is TimeoutException or TaskCanceledException)
        {
            Log.Info("The Mac did not send its clipboard data");
        }
        finally
        {
            deferral.Complete();
        }
    }

    // The kinds the Mac can take from the Windows clipboard. Another app may hold it open, in which case reading fails
    // until it lets go.
    private static List<string> Kinds()
    {
        try
        {
            var content = SystemClipboard.GetContent();
            var kinds = new List<string>();
            if (content.Contains(StandardDataFormats.Text)) kinds.Add(Text);
            if (content.Contains(StandardDataFormats.Bitmap)) kinds.Add(Png);
            return kinds;
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
        {
            Log.Info("Cannot read the clipboard");
            return [];
        }
    }

    // Reads the data of the offered kinds, with text in the Mac's form and images as PNG.
    private static async Task<List<ClipboardItem>> Read(List<string> kinds)
    {
        var items = new List<ClipboardItem>();

        try
        {
            var content = SystemClipboard.GetContent();

            if (kinds.Contains(Text) && content.Contains(StandardDataFormats.Text))
            {
                var text = (await content.GetTextAsync()).ReplaceLineEndings("\n");
                items.Add(new ClipboardItem(Text, Encoding.UTF8.GetBytes(text)));
            }

            if (kinds.Contains(Png) && content.Contains(StandardDataFormats.Bitmap))
            {
                using var source = await (await content.GetBitmapAsync()).OpenReadAsync();
                items.Add(new ClipboardItem(Png, await Encode(source)));
            }
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
        {
            Log.Info("Cannot read the clipboard");
        }

        return items;
    }

    // Re-encodes a clipboard image, which Windows holds as a bitmap, as PNG.
    private static async Task<byte[]> Encode(IRandomAccessStream source)
    {
        var decoder = await BitmapDecoder.CreateAsync(source);
        using var bitmap = await decoder.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied);
        using var output = new InMemoryRandomAccessStream();
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, output);
        encoder.SetSoftwareBitmap(bitmap);
        await encoder.FlushAsync();

        var bytes = new byte[output.Size];
        output.Seek(0);
        await output.ReadAsync(bytes.AsBuffer(), (uint)bytes.Length, InputStreamOptions.None);
        return bytes;
    }

    private static async Task<IRandomAccessStream> Stream(byte[] data)
    {
        var stream = new InMemoryRandomAccessStream();
        await stream.WriteAsync(data.AsBuffer());
        stream.Seek(0);
        return stream;
    }

    private static byte[]? Find(List<ClipboardItem> items, string kind) => items.Find(item => item.Kind == kind)?.Data;

    private static string Describe(IEnumerable<ClipboardItem> items) =>
        string.Join(", ", items.Select(item => $"{item.Kind} {item.Data.Length} bytes"));
}
