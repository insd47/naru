using System.Diagnostics;

namespace Naru;

/// <summary>
/// Writes diagnostics to the debugger and trace listeners.
/// </summary>
/// <remarks>Never pass passwords, keys, or clipboard contents.</remarks>
internal static class Log
{
    /// <summary>Also writes to naru.log in the app's local folder, replaced on each launch, so a session can be traced without a debugger.</summary>
    public static void ToFile()
    {
        var path = Path.Combine(Windows.Storage.ApplicationData.Current.LocalFolder.Path, "naru.log");
        Trace.Listeners.Add(new TextWriterTraceListener(new StreamWriter(path, append: false)));
        Trace.AutoFlush = true;
    }

    public static void Info(string message) => Trace.WriteLine($"[Naru] {DateTime.Now:HH:mm:ss.fff} {message}");

    public static void Error(string message, Exception error) => Trace.WriteLine($"[Naru] {DateTime.Now:HH:mm:ss.fff} {message}: {error}");
}
