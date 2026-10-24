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
    // Command goes as Alt and Option as Meta, as Apple's client sends them.
    private const uint CommandLeft = 0xffe9;
    private const uint CommandRight = 0xffea;
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

    // Scan codes (set 1, as the keyboard hook reports them, with 0x100 for the extended ones) of physical keys and their
    // USB HID usages, per Microsoft's keyboard scan code specification. The core turns a usage into the Mac's key code.
    private static readonly Dictionary<uint, ushort> Usages = new()
    {
        [0x1e] = 0x04, [0x30] = 0x05, [0x2e] = 0x06, [0x20] = 0x07, [0x12] = 0x08, [0x21] = 0x09, [0x22] = 0x0a,
        [0x23] = 0x0b, [0x17] = 0x0c, [0x24] = 0x0d, [0x25] = 0x0e, [0x26] = 0x0f, [0x32] = 0x10, [0x31] = 0x11,
        [0x18] = 0x12, [0x19] = 0x13, [0x10] = 0x14, [0x13] = 0x15, [0x1f] = 0x16, [0x14] = 0x17, [0x16] = 0x18,
        [0x2f] = 0x19, [0x11] = 0x1a, [0x2d] = 0x1b, [0x15] = 0x1c, [0x2c] = 0x1d,
        [0x02] = 0x1e, [0x03] = 0x1f, [0x04] = 0x20, [0x05] = 0x21, [0x06] = 0x22, [0x07] = 0x23, [0x08] = 0x24,
        [0x09] = 0x25, [0x0a] = 0x26, [0x0b] = 0x27,
        [0x1c] = 0x28, [0x01] = 0x29, [0x0e] = 0x2a, [0x0f] = 0x2b, [0x39] = 0x2c, [0x0c] = 0x2d, [0x0d] = 0x2e,
        [0x1a] = 0x2f, [0x1b] = 0x30, [0x2b] = 0x31, [0x27] = 0x33, [0x28] = 0x34, [0x29] = 0x35, [0x33] = 0x36,
        [0x34] = 0x37, [0x35] = 0x38, [0x3a] = 0x39,
        [0x3b] = 0x3a, [0x3c] = 0x3b, [0x3d] = 0x3c, [0x3e] = 0x3d, [0x3f] = 0x3e, [0x40] = 0x3f, [0x41] = 0x40,
        [0x42] = 0x41, [0x43] = 0x42, [0x44] = 0x43, [0x57] = 0x44, [0x58] = 0x45,
        [0x152] = 0x49, [0x147] = 0x4a, [0x149] = 0x4b, [0x153] = 0x4c, [0x14f] = 0x4d, [0x151] = 0x4e,
        [0x14d] = 0x4f, [0x14b] = 0x50, [0x150] = 0x51, [0x148] = 0x52,
        [0x45] = 0x53, [0x135] = 0x54, [0x37] = 0x55, [0x4a] = 0x56, [0x4e] = 0x57, [0x11c] = 0x58,
        [0x4f] = 0x59, [0x50] = 0x5a, [0x51] = 0x5b, [0x4b] = 0x5c, [0x4c] = 0x5d, [0x4d] = 0x5e, [0x47] = 0x5f,
        [0x48] = 0x60, [0x49] = 0x61, [0x52] = 0x62, [0x53] = 0x63, [0x56] = 0x64,
        [0x1d] = 0xe0, [0x2a] = 0xe1, [0x38] = 0xe2, [0x15b] = 0xe3, [0x11d] = 0xe4, [0x36] = 0xe5, [0x138] = 0xe6,
        [0x15c] = 0xe7,
    };

    // The keysym and usage each held key went with
    private readonly Dictionary<uint, (uint Keysym, ushort? Usage)> pressed = [];

    private readonly Dictionary<uint, uint> modifiers = swapCommandOption
        ? new() { [VkLMenu] = CommandLeft, [VkRMenu] = CommandRight, [VkLWin] = OptionLeft, [VkRWin] = OptionRight }
        : new() { [VkLMenu] = OptionLeft, [VkRMenu] = OptionRight, [VkLWin] = CommandLeft, [VkRWin] = CommandRight };

    /// <summary>
    /// The keys to send for a key, empty for a key the Mac does not get: the keysym, the physical key as a USB HID usage,
    /// and whether it is pressed.
    /// </summary>
    /// <param name="held">Keys held, this one included</param>
    /// <param name="language">The input language as a BCP 47 tag</param>
    /// <param name="source">The Mac's latest input source</param>
    public List<(uint Keysym, ushort? Usage, bool Down)> Translate(uint vk, uint scan, bool extended, bool down, nint layout, IReadOnlySet<uint> held, string language, InputSource? source)
    {
        // The extended flag keeps the keypad Enter and the main Enter apart.
        var key = vk | (extended ? 0x100u : 0);

        if (!down)
        {
            return pressed.Remove(key, out var sent) ? [(sent.Keysym, sent.Usage, false)] : [];
        }

        // Auto-repeat resends the keysym pressed first.
        if (pressed.TryGetValue(key, out var repeat))
        {
            return [(repeat.Keysym, repeat.Usage, true)];
        }

        var characters = Keysyms(vk, scan, layout, held, language, source);

        if (characters.Count == 0)
        {
            return [];
        }

        // A dead key that did not compose sends both characters, the first as no key of its own. Only the last stays held.
        var usage = Usages.TryGetValue(scan | (extended ? 0x100u : 0), out var found) ? found : (ushort?)null;
        pressed[key] = (characters[^1], usage);
        var result = new List<(uint, ushort?, bool)>();

        for (var index = 0; index < characters.Count - 1; index++)
        {
            result.Add((characters[index], null, true));
            result.Add((characters[index], null, false));
        }

        result.Add((characters[^1], usage, true));
        return result;
    }

    /// <summary>Releases every pressed key so none stays held on the Mac when the window loses the keyboard.</summary>
    public List<(uint Keysym, ushort? Usage, bool Down)> ReleaseAll()
    {
        var released = pressed.Values.Select(sent => (sent.Keysym, sent.Usage, false)).ToList();
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
