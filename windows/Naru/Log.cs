using System.Diagnostics;

namespace Naru;

/// <summary>
/// Writes diagnostics to the debugger and trace listeners.
/// </summary>
/// <remarks>Never pass passwords, keys, or clipboard contents.</remarks>
internal static class Log
{
    public static void Info(string message) => Trace.WriteLine($"[Naru] {message}");

    public static void Error(string message, Exception error) => Trace.WriteLine($"[Naru] {message}: {error}");
}
