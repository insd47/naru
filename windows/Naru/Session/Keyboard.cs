using System.Runtime.InteropServices;
using Naru.Store;
using static Naru.Session.Native;

namespace Naru.Session;

/// <summary>
/// Captures the keyboard for the session window in front with a low-level hook, as Remote Desktop does.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The hook sees keys before Windows and the IME, so shortcuts reach the Mac and the IME does not compose locally.</item>
/// <item>Windows key combinations (the Windows key, Alt+Tab, Alt+Esc, Ctrl+Esc) stay with Windows unless the setting sends them to the Mac.</item>
/// <item>Ctrl+Alt+Break toggles full screen, as in Remote Desktop.</item>
/// <item>The Korean/English key never reaches the IME, so the Korean mode is tracked here as the input language that decides what keys type.</item>
/// <item>With Sync Keyboard Language on, the Mac's input source follows that language. Switching also commits what the Mac's
/// input method is still composing.</item>
/// <item>The hook runs on the UI thread that installed it and must return quickly, or Windows removes it.</item>
/// </list>
/// </remarks>
internal sealed class Keyboard : IDisposable
{
    private const int KoreanLanguage = 0x0412;
    // Scan code of the right Alt, which most Korean keyboards report as the Hangul key
    private const uint RightAltScan = 0x38;

    private static readonly Dictionary<nint, Keyboard> Attached = [];
    private static nint hook;

    private readonly nint window;
    private readonly Connection connection;
    private readonly Preferences preferences;
    private readonly Func<bool> capturing;
    private readonly Func<bool> fullScreen;
    private readonly Action toggleFullScreen;
    // Keys held, tracked here because the hook swallows them before Windows updates its own key state
    private readonly HashSet<uint> held = [];
    private Keys keys;
    private bool korean;
    private string language = "";
    // The Mac input source last asked for
    private string? asked;

    public Keyboard(nint window, Connection connection, Preferences preferences, Func<bool> capturing, Func<bool> fullScreen, Action toggleFullScreen)
    {
        this.window = window;
        this.connection = connection;
        this.preferences = preferences;
        this.capturing = capturing;
        this.fullScreen = fullScreen;
        this.toggleFullScreen = toggleFullScreen;
        keys = new Keys(preferences.SwapCommandOption);
        korean = ImeNative(window);

        Attached[window] = this;
        Install();
    }

    /// <summary>The window came to the front: settings may have changed, and the language is read and selected again.</summary>
    public void Activated()
    {
        keys = new Keys(preferences.SwapCommandOption);
        held.Clear();
        asked = null;
        Report(Layout());
    }

    /// <summary>The window lost the keyboard. Held keys are released so none stays held on the Mac.</summary>
    public void Deactivated()
    {
        Send(keys.ReleaseAll());
        held.Clear();
    }

    public void Dispose()
    {
        Attached.Remove(window);

        if (Attached.Count == 0 && hook != 0)
        {
            Native.UnhookWindowsHookEx(hook);
            hook = 0;
        }
    }

    private static unsafe void Install()
    {
        if (hook == 0)
        {
            hook = Native.SetWindowsHookEx(WhKeyboardLl, &Hook, Native.GetModuleHandle(null), 0);

            if (hook == 0)
            {
                Log.Info($"Cannot install the keyboard hook: error {Marshal.GetLastPInvokeError()}");
            }
        }
    }

    [UnmanagedCallersOnly]
    private static unsafe nint Hook(int code, nint message, nint data)
    {
        if (code >= 0 && Attached.TryGetValue(Native.GetForegroundWindow(), out var target))
        {
            var key = (Native.KbdLlHookStruct*)data;
            var down = message is Native.WmKeyDown or Native.WmSysKeyDown;

            try
            {
                if ((key->Flags & Native.LlkhfInjected) == 0 && target.Handle(key->VkCode, key->ScanCode, (key->Flags & Native.LlkhfExtended) != 0, down))
                {
                    return 1;
                }
            }
            catch (Exception error)
            {
                // An exception must not cross into Windows, which would end the process.
                Log.Error("Keyboard hook failed", error);
            }
        }

        return Native.CallNextHookEx(hook, code, message, data);
    }

    // Returns whether the key is swallowed. Keys pass to Windows while a menu or dialog has the window's attention.
    private bool Handle(uint vk, uint scan, bool extended, bool down)
    {
        if (down)
        {
            held.Add(vk);
        }
        else
        {
            held.Remove(vk);
        }

        if (!capturing())
        {
            return false;
        }

        var control = held.Overlaps([VkLControl, VkRControl]);
        var alt = held.Overlaps([VkLMenu, VkRMenu]);

        // Ctrl+Pause arrives as Cancel.
        if (vk is VkCancel or VkPause && control && alt)
        {
            if (down)
            {
                toggleFullScreen();
            }

            return true;
        }

        var layout = Layout();

        // The Hangul key goes to the Mac as the key it is, mostly the right Alt. Like Apple's client with Caps Lock, the
        // current source is asked for while it is held, so its release changes nothing, and the new mode's after the
        // release, so the first typed key switches.
        if (vk == VkHangul && (layout & 0xffff) == KoreanLanguage)
        {
            Send(keys.Translate(scan == RightAltScan ? VkRMenu : vk, scan, extended, down, layout, held, language, connection.Source));

            if (down)
            {
                Sync(again: true);
                korean = !korean;
            }
            else
            {
                Report(layout);
            }

            return true;
        }

        if (vk == VkHanja)
        {
            return true;
        }

        var local = preferences.WindowsKeys switch
        {
            WindowsKeys.ThisComputer => true,
            WindowsKeys.FullScreenOnly => !fullScreen(),
            _ => false,
        };

        if (local && (vk is VkLWin or VkRWin || (alt && vk is VkTab or VkEscape) || (control && vk == VkEscape)))
        {
            return false;
        }

        // Alt also goes to Windows, which needs it for Alt+Tab. The Mac still gets it as Command.
        var shared = local && vk is VkLMenu or VkRMenu;

        if (!shared)
        {
            Report(layout);
        }

        Send(keys.Translate(vk, scan, extended, down, layout, held, language, connection.Source));
        return !shared;
    }

    private void Send(List<(uint Keysym, bool Down)> events)
    {
        foreach (var (keysym, down) in events)
        {
            connection.Key(keysym, down);
        }
    }

    // A Korean layout types English or Korean depending on its mode. Other layouts report their own language.
    private void Report(nint layout)
    {
        var next = (layout & 0xffff) == KoreanLanguage ? (korean ? "ko-KR" : "en-US") : Keys.LanguageOf(layout);

        if (next != language)
        {
            language = next;
            Log.Info($"Input language: {language}");
        }

        Sync();
    }

    // The source goes out when the language changes. A source already asked for is asked again only for the Hangul key,
    // as Apple's client does with Caps Lock, so a source the Mac changed to by itself, as when another window took focus,
    // is left alone until the language changes, as Apple's client leaves it.
    private void Sync(bool again = false)
    {
        // Turned off, the Mac keeps its own source again, as with Apple's client; the agent would otherwise undo a switch
        // made on the Mac at every key.
        if (!preferences.SyncKeyboardLanguage && asked is not null)
        {
            connection.ReleaseInputSource();
            asked = null;
        }

        if (preferences.SyncKeyboardLanguage && language.Length > 0 && Core.NaruMethods.InputSource(language, connection.Source) is { } source && (again || source != asked))
        {
            connection.SelectInputSource(source);
            asked = source;
        }
    }

    private nint Layout() => Native.GetKeyboardLayout(Native.GetWindowThreadProcessId(window, 0));

    private static bool ImeNative(nint window)
    {
        var context = Native.ImmGetContext(window);

        if (context == 0)
        {
            return false;
        }

        try
        {
            return Native.ImmGetConversionStatus(context, out var conversion, out _) && (conversion & Native.ImeCmodeNative) != 0;
        }
        finally
        {
            Native.ImmReleaseContext(window, context);
        }
    }
}
