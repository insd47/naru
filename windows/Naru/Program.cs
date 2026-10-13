using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace Naru;

/// <summary>
/// The entry point, which keeps Naru to one instance.
/// </summary>
/// <remarks>
/// A second launch, such as from a jump list item, hands its activation to the running instance and exits, so the
/// running app opens the connection and the saved lists are never edited by two processes. This follows the Windows
/// App SDK's single-instance pattern, which waits for the redirect while still pumping COM.
/// </remarks>
public static partial class Program
{
    [STAThread]
    private static int Main()
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        var main = AppInstance.FindOrRegisterForKey("main");

        if (!main.IsCurrent)
        {
            Redirect(main);
            return 0;
        }

        Application.Start(_ =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            var app = new App();
        });

        return 0;
    }

    private static unsafe void Redirect(AppInstance main)
    {
        var arguments = AppInstance.GetCurrent().GetActivatedEventArgs();
        var done = CreateEvent(0, true, false, 0);

        Task.Run(() =>
        {
            main.RedirectActivationToAsync(arguments).AsTask().Wait();
            SetEvent(done);
        });

        var handles = stackalloc nint[] { done };
        CoWaitForMultipleObjects(0, uint.MaxValue, 1, handles, out _);
        CloseHandle(done);

        // The launching process may bring a window forward; the running one may not on its own.
        SetForegroundWindow(Process.GetProcessById((int)main.ProcessId).MainWindowHandle);
    }

    [LibraryImport("kernel32.dll", EntryPoint = "CreateEventW")]
    private static partial nint CreateEvent(nint attributes, [MarshalAs(UnmanagedType.Bool)] bool manualReset, [MarshalAs(UnmanagedType.Bool)] bool initialState, nint name);

    [LibraryImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SetEvent(nint handle);

    [LibraryImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CloseHandle(nint handle);

    [LibraryImport("ole32.dll")]
    private static unsafe partial int CoWaitForMultipleObjects(uint flags, uint timeout, uint count, nint* handles, out uint index);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SetForegroundWindow(nint window);
}
