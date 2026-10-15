using System.Diagnostics;

namespace Naru;

/// <summary>
/// Writes diagnostics to the debugger and trace listeners.
/// </summary>
/// <remarks>Never pass passwords, keys, or clipboard contents.</remarks>
internal static class Log
{
    /// <summary>The log file in the app's local folder, replaced on each launch</summary>
    public static string Path => System.IO.Path.Combine(Windows.Storage.ApplicationData.Current.LocalFolder.Path, "naru.log");

    /// <summary>Also writes to <see cref="Path"/>, so a session can be traced without a debugger.</summary>
    public static void ToFile()
    {
        Trace.Listeners.Add(new TextWriterTraceListener(new StreamWriter(Path, append: false)));
        Trace.AutoFlush = true;
    }

    public static void Info(string message) => Trace.WriteLine($"[Naru] {DateTime.Now:HH:mm:ss.fff} {message}");

    public static void Error(string message, Exception error) => Trace.WriteLine($"[Naru] {DateTime.Now:HH:mm:ss.fff} {message}: {error}");
}
