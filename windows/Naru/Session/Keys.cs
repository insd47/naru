using System.Globalization;
using Naru.Core;
using static Naru.Session.Native;

namespace Naru.Session;

/// <summary>
/// Converts Windows virtual keys to X11 keysyms, following the Android app.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Modifiers map by position. On a PC keyboard the Alt next to the space bar sits where Command is on a Mac, so Alt → Command and
/// the Windows key → Option. When <c>swapCommandOption</c> is off, they map by name (Alt → Option, Windows → Command).</item>
/// <item>Character keys type what the current Windows layout's character means in the input language, through the core's
/// <c>Character</c>: Dubeolsik jamo for Korean, following the Mac's secure input and its own layout of that language. Other languages
/// send the character as is.</item>
/// <item>Without Control, Option, or Command held, sends the shifted character (<c>A</c>, <c>!</c>); with any held, the unshifted one (<c>c</c>).</item>
/// <item>A release sends the same keysym as the press, so no key stays held on the Mac if Shift changed in between.</item>
/// <item>Dead keys (<c>´</c> then <c>e</c>) send the composed character (<c>é</c>).</item>
/// <item>Caps Lock and the Hangul key are sent as they are, as Apple's client sends its keys. The Mac does not switch input sources on a
/// remote key.</item>
/// </list>
/// </remarks>
internal sealed unsafe class Keys(bool swapCommandOption)
{
    private const uint CommandLeft = 0xffeb;
    private const uint CommandRight = 0xffec;
    private const uint OptionLeft = 0xffe7;
    private const uint OptionRight = 0xffe8;

    private static readonly HashSet<uint> ShortcutModifiers = [VkLControl, VkRControl, VkLMenu, VkRMenu, VkLWin, VkRWin];

    private static readonly Dictionary<uint, uint> Special = new()
    {
        [VkLShift] = 0xffe1,
        [VkRShift] = 0xffe2,
        [VkLControl] = 0xffe3,
        [VkRControl] = 0xffe4,
        [VkEscape] = 0xff1b,
        [VkTab] = 0xff09,
        [VkBack] = 0xff08,
        [VkReturn] = 0xff0d,
        [VkDelete] = 0xffff,
        [VkInsert] = 0xff63,
        [VkHome] = 0xff50,
        [VkEnd] = 0xff57,
        [VkPrior] = 0xff55,
        [VkNext] = 0xff56,
        [VkLeft] = 0xff51,
        [VkUp] = 0xff52,
        [VkRight] = 0xff53,
        [VkDown] = 0xff54,
        [VkSpace] = 0x20,
        [VkCapital] = 0xffe5,
        [VkHangul] = 0xff31,
    };

    private readonly Dictionary<uint, uint> pressed = [];

    private readonly Dictionary<uint, uint> modifiers = swapCommandOption
        ? new() { [VkLMenu] = CommandLeft, [VkRMenu] = CommandRight, [VkLWin] = OptionLeft, [VkRWin] = OptionRight }
        : new() { [VkLMenu] = OptionLeft, [VkRMenu] = OptionRight, [VkLWin] = CommandLeft, [VkRWin] = CommandRight };

    /// <summary>
    /// The (keysym, down) pairs to send for a key, empty for a key the Mac does not get.
    /// </summary>
    /// <param name="held">Keys held, this one included</param>
    /// <param name="language">The input language as a BCP 47 tag</param>
    /// <param name="source">The Mac's latest input source</param>
    public List<(uint Keysym, bool Down)> Translate(uint vk, uint scan, bool extended, bool down, nint layout, IReadOnlySet<uint> held, string language, InputSource? source)
    {
        // The extended flag keeps the keypad Enter and the main Enter apart.
        var key = vk | (extended ? 0x100u : 0);

        if (!down)
        {
            return pressed.Remove(key, out var sent) ? [(sent, false)] : [];
        }

        // Auto-repeat resends the keysym pressed first.
        if (pressed.TryGetValue(key, out var repeat))
        {
            return [(repeat, true)];
        }

        var characters = Keysyms(vk, scan, layout, held, language, source);

        if (characters.Count == 0)
        {
            return [];
        }

        // A dead key that did not compose sends both characters. Only the last stays held.
        pressed[key] = characters[^1];
        var result = new List<(uint, bool)>();

        for (var index = 0; index < characters.Count - 1; index++)
        {
            result.Add((characters[index], true));
            result.Add((characters[index], false));
        }

        result.Add((characters[^1], true));
        return result;
    }

    /// <summary>Releases every pressed key so none stays held on the Mac when the window loses the keyboard.</summary>
    public List<(uint Keysym, bool Down)> ReleaseAll()
    {
        var released = pressed.Values.Select(keysym => (keysym, false)).ToList();
        pressed.Clear();
        return released;
    }

    /// <summary>The input language of a layout as a BCP 47 tag, or empty if unknown.</summary>
    public static string LanguageOf(nint layout)
    {
        try
        {
            return CultureInfo.GetCultureInfo((int)(layout & 0xffff)).Name;
        }
        catch (CultureNotFoundException)
        {
            return "";
        }
    }

    private List<uint> Keysyms(uint vk, uint scan, nint layout, IReadOnlySet<uint> held, string language, InputSource? source)
    {
        if (modifiers.TryGetValue(vk, out var modifier))
        {
            return [modifier];
        }

        if (Special.TryGetValue(vk, out var special))
        {
            return [special];
        }

        // F1 to F24
        if (vk is >= 0x70 and <= 0x87)
        {
            return [0xffbe + vk - 0x70];
        }

        var keysyms = Characters(vk, scan, layout, held).Select(ToKeysym);

        // Shortcuts send the Latin key itself.
        if (held.Overlaps(ShortcutModifiers))
        {
            return keysyms.ToList();
        }

        return keysyms.Select(keysym => NaruMethods.Character(keysym, language, source) ?? keysym).ToList();
    }

    // Latin-1 uses the character code directly; anything else uses a Unicode keysym.
    private static uint ToKeysym(char character) => character is >= (char)0x20 and <= (char)0x7e or >= (char)0xa0 and <= (char)0xff
        ? character
        : 0x0100_0000u + character;

    // The layout's characters for the key. Only Shift counts, unless a shortcut modifier is held. A pending dead key composes with the next key.
    private static List<char> Characters(uint vk, uint scan, nint layout, IReadOnlySet<uint> held)
    {
        var state = stackalloc byte[256];
        var shortcut = held.Overlaps(ShortcutModifiers);

        if (!shortcut && held.Overlaps([VkLShift, VkRShift]))
        {
            state[VkShift] = 0x80;
        }

        var buffer = stackalloc char[8];
        var count = ToUnicodeEx(vk, scan, state, buffer, 8, 0, layout);

        // A dead key returns -1 and waits for the next key, which returns the composed character.
        return count <= 0 ? [] : new string(buffer, 0, count).Where(character => !char.IsControl(character)).ToList();
    }
}
