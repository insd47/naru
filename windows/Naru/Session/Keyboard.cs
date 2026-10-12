using System.Runtime.InteropServices;
using Naru.Store;

namespace Naru.Session;

/// <summary>
/// Captures the keyboard for the session window in front with a low-level hook, as Remote Desktop does.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The hook sees keys before Windows and the IME, so shortcuts reach the Mac and the IME does not compose locally.</item>
/// <item>Windows key combinations (the Windows key, Alt+Tab, Alt+Esc, Ctrl+Esc) stay with Windows unless the setting sends them to the Mac.</item>
/// <item>Ctrl+Alt+Break toggles full screen, as in Remote Desktop.</item>
/// <item>The Korean/English key never reaches the IME, so the Korean mode is tracked here and reported to the session as the input language.</item>
/// <item>The hook runs on the UI thread that installed it and must return quickly, or Windows removes it.</item>
/// </list>
/// </remarks>
internal sealed class Keyboard
{
    private const uint VkCancel = 0x03;
    private const uint VkTab = 0x09;
    private const uint VkPause = 0x13;
    private const uint VkHangul = 0x15;
    private const uint VkHanja = 0x19;
    private const uint VkEscape = 0x1b;
    private const uint VkLWin = 0x5b;
    private const uint VkRWin = 0x5c;
    private const uint VkLControl = 0xa2;
    private const uint VkRControl = 0xa3;
    private const uint VkLMenu = 0xa4;
    private const uint VkRMenu = 0xa5;
    private const int KoreanLanguage = 0x0412;

    private static readonly Dictionary<nint, Keyboard> Windows = [];
    private static nint hook;

    private readonly nint window;
    private readonly Connection connection;
    private readonly Preferences preferences;
    private readonly Func<bool> capturing;
    private readonly Func<bool> fullScreen;
    private readonly Action toggleFullScreen;
    private readonly HashSet<uint> held = [];
    private Keys keys;
    private bool korean;
    private string? reported;

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

        Windows[window] = this;
        Install();
    }

    /// <summary>The window came to the front: settings may have changed, and the language is reported again.</summary>
    public void Activated()
    {
        keys = new Keys(preferences.SwapCommandOption);
        held.Clear();
        reported = null;
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
        Windows.Remove(window);

        if (Windows.Count == 0 && hook != 0)
        {
            Native.UnhookWindowsHookEx(hook);
            hook = 0;
        }
    }

    private static unsafe void Install()
    {
        if (hook == 0)
        {
            hook = Native.SetWindowsHookEx(Native.WhKeyboardLl, &Hook, Native.GetModuleHandle(null), 0);

            if (hook == 0)
            {
                Log.Info($"Cannot install the keyboard hook: error {Marshal.GetLastPInvokeError()}");
            }
        }
    }

    [UnmanagedCallersOnly]
    private static unsafe nint Hook(int code, nint message, nint data)
    {
        if (code >= 0 && Windows.TryGetValue(Native.GetForegroundWindow(), out var target))
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

        if (vk == VkHangul && (layout & 0xffff) == KoreanLanguage)
        {
            if (down)
            {
                korean = !korean;
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

        if (local)
        {
            if (vk is VkLWin or VkRWin || (alt && vk is VkTab or VkEscape) || (control && vk == VkEscape))
            {
                return false;
            }

            // Alt also goes to Windows, which needs it for Alt+Tab. The Mac still gets it as Command.
            if (vk is VkLMenu or VkRMenu)
            {
                Send(keys.Translate(vk, scan, extended, down, layout));
                return false;
            }
        }

        Report(layout);
        Send(keys.Translate(vk, scan, extended, down, layout));
        return true;
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
        var language = (layout & 0xffff) == KoreanLanguage ? (korean ? "ko-KR" : "en-US") : Keys.LanguageOf(layout);

        if (language != reported)
        {
            reported = language;
            Log.Info($"Input language: {language}");
            connection.Language(language);
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
