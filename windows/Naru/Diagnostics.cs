using System.Runtime.InteropServices;
using Microsoft.UI;
using Microsoft.Win32;
using Microsoft.Windows.Storage.Pickers;
using Vortice.DXGI;
using Windows.ApplicationModel;

namespace Naru;

/// <summary>
/// Exports this launch's log as a text file for a bug report.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The log is naru.log, which <see cref="Log"/> replaces on each launch, so it covers this launch only.</item>
/// <item>A header describes the app, the system, and the hardware that decides how video is decoded.</item>
/// <item>Nothing the app logs carries passwords, keys, typed text, or clipboard contents.</item>
/// </list>
/// </remarks>
internal static class Diagnostics
{
    /// <summary>Asks where to save the file and writes it there. Returns whether it was saved.</summary>
    public static async Task<bool> Export(WindowId window)
    {
        var picker = new FileSavePicker(window)
        {
            SuggestedFileName = $"Naru Diagnostics {DateTime.Now:yyyy-MM-dd HHmmss}",
            DefaultFileExtension = ".txt",
        };
        picker.FileTypeChoices.Add("Text", [".txt"]);

        if (await picker.PickSaveFileAsync() is not { } result)
        {
            return false;
        }

        var hevc = await Task.Run(Video.Decoder.SupportsHevc444);
        var text = await Task.Run(() => Header(hevc) + Read());
        await File.WriteAllTextAsync(result.Path, text);
        Log.Info("Diagnostics exported");
        return true;
    }

    private static string Header(bool hevc)
    {
        var version = Package.Current.Id.Version;

        return string.Join(Environment.NewLine,
            $"Naru {version.Major}.{version.Minor}.{version.Build}, Windows {Environment.OSVersion.Version} ({RuntimeInformation.OSArchitecture})",
            $"{Processor()}, {Adapter()}",
            $"HEVC 4:4:4 hardware decoding: {(hevc ? "supported" : "unsupported")}",
            $"Exported {DateTimeOffset.Now:yyyy-MM-dd HH:mm:ss zzz}",
            "",
            "");
    }

    // The log stays open for writing while the app runs.
    private static string Read()
    {
        try
        {
            using var stream = new FileStream(Log.Path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            using var reader = new StreamReader(stream);
            return reader.ReadToEnd();
        }
        catch (IOException error)
        {
            return $"Cannot read the log: {error.Message}";
        }
    }

    private static string Processor()
    {
        using var key = Registry.LocalMachine.OpenSubKey(@"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        return (key?.GetValue("ProcessorNameString") as string)?.Trim() ?? "Unknown processor";
    }

    // The default adapter, which the video decoder uses
    private static string Adapter()
    {
        try
        {
            using var factory = DXGI.CreateDXGIFactory1<IDXGIFactory1>();
            factory.EnumAdapters1(0, out var adapter).CheckError();
            using (adapter)
            {
                return adapter.Description1.Description;
            }
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            return "Unknown graphics adapter";
        }
    }
}
