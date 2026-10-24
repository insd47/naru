package dev.insd.naru.session

import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import dev.insd.naru.core.InputSource
import dev.insd.naru.core.character

/**
 * Converts Android key events to X11 keysyms.
 *
 * - Modifiers map by position. On a PC layout the Alt next to the space bar sits where Command is on a Mac, so Alt → Command and
 *   Meta → Option. When [swapCommandOption] is `false`, they map by name (Alt → Option, Meta → Command).
 * - The Mac treats Super_L as Command and Meta_L as Option. It also treats Alt_L as Command, so Alt_L is not used for Option.
 * - Character keys type what the physical key's Latin character means in the IME's language, through the core's [character]: Dubeolsik jamo for
 *   Korean, following the Mac's secure input and its own layout of that language. Other languages send the Latin character as is.
 * - Without Control, Option, or Command held, sends the shifted character (`A`, `!`); with any held, sends the lowercase one (`c`).
 * - A release sends the same keysym as the press, so no key stays held on the Mac if Shift changed in between.
 * - Dead keys (`´` then `e`) send the character composed by the tablet layout (`é`). The Mac inserts characters missing from its layout as is.
 * - Caps Lock is not sent. Case is already in the character, and if Caps Lock switches the Mac to ABC, Korean comes out as separate jamo.
 */
class Keys(swapCommandOption: Boolean) {
    /** A key to send: the keysym, the physical key as a USB HID usage or `null`, and whether it is pressed */
    data class Key(val keysym: Int, val usage: Int?, val down: Boolean)

    // The keysym and usage each held key went with
    private val pressed = HashMap<Int, Key>()

    // Combining character of the pending dead key, composed with the next character.
    private var accent = 0

    private val modifiers = if (swapCommandOption) {
        mapOf(
            KeyEvent.KEYCODE_ALT_LEFT to COMMAND_LEFT,
            KeyEvent.KEYCODE_ALT_RIGHT to COMMAND_RIGHT,
            KeyEvent.KEYCODE_META_LEFT to OPTION_LEFT,
            KeyEvent.KEYCODE_META_RIGHT to OPTION_RIGHT,
        )
    } else {
        mapOf(
            KeyEvent.KEYCODE_ALT_LEFT to OPTION_LEFT,
            KeyEvent.KEYCODE_ALT_RIGHT to OPTION_RIGHT,
            KeyEvent.KEYCODE_META_LEFT to COMMAND_LEFT,
            KeyEvent.KEYCODE_META_RIGHT to COMMAND_RIGHT,
        )
    }

    /** The keys to send, empty for an unknown key. [language] is the IME's, and [source] the Mac's latest input source. */
    fun translate(event: KeyEvent, language: String, source: InputSource?): List<Key> {
        val down = when (event.action) {
            KeyEvent.ACTION_DOWN -> true
            KeyEvent.ACTION_UP -> false
            else -> return emptyList()
        }

        if (!down) {
            return pressed.remove(event.keyCode)?.let { listOf(it.copy(down = false)) } ?: emptyList()
        }

        val keysym = keysym(event, language, source) ?: return emptyList()
        val key = Key(keysym, USAGES[event.scanCode], down = true)
        pressed[event.keyCode] = key
        return listOf(key)
    }

    /**
     * Whether the key came from a mouse back/forward button.
     *
     * - Android turns these buttons into BACK/FORWARD keys, so passing them to the system would leave the session screen. They are swallowed.
     * - Screen Sharing has no such buttons, so they are not sent to the Mac either.
     */
    fun isMouseNavigation(event: KeyEvent): Boolean =
        (event.keyCode == KeyEvent.KEYCODE_BACK || event.keyCode == KeyEvent.KEYCODE_FORWARD) &&
            event.isFromSource(InputDevice.SOURCE_MOUSE)

    /** Whether this is a dead key. It is swallowed rather than passed to the system, since it is sent composed with the next character. */
    fun isDeadKey(event: KeyEvent): Boolean = char(event) and KeyCharacterMap.COMBINING_ACCENT != 0

    /** Releases every held key so none stays held on the Mac when leaving the screen. */
    fun releaseAll(): List<Key> = pressed.values.map { it.copy(down = false) }.also { pressed.clear() }

    private fun keysym(event: KeyEvent, language: String, source: InputSource?): Int? {
        modifiers[event.keyCode]?.let { return it }

        // A space or arrow after a dead key discards the dead key. Shift is kept, since it combines as in `É`.
        SPECIAL[event.keyCode]?.let { keysym ->
            if (!KeyEvent.isModifierKey(event.keyCode)) accent = 0
            return keysym
        }

        val char = char(event)

        if (char == 0) {
            return null
        }

        if (char and KeyCharacterMap.COMBINING_ACCENT != 0) {
            accent = char and KeyCharacterMap.COMBINING_ACCENT_MASK
            return null
        }

        // If the character cannot be composed, the dead key is discarded and only the character is sent.
        val combined = KeyCharacterMap.getDeadChar(accent, char).takeIf { accent != 0 && it != 0 } ?: char
        accent = 0

        // Latin-1 uses the character code directly; anything else uses a Unicode keysym.
        val latin = if (combined in 0x20..0x7e || combined in 0xa0..0xff) combined else 0x0100_0000 + combined

        // Shortcuts send the Latin key itself.
        if (event.metaState and SHORTCUT != 0) return latin

        return character(latin.toUInt(), language, source)?.toInt() ?: latin
    }

    private fun char(event: KeyEvent): Int {
        val meta = if (event.metaState and SHORTCUT != 0) 0 else event.metaState and KeyEvent.META_SHIFT_MASK
        return event.keyCharacterMap.get(event.keyCode, meta)
    }

    companion object {
        private const val SHORTCUT = KeyEvent.META_CTRL_ON or KeyEvent.META_ALT_ON or KeyEvent.META_META_ON

        // Super_L/R are confirmed as Command and Meta_L as Option. Meta_R mirrors Meta_L and is unverified.
        // Command goes as Alt and Option as Meta, as Apple's client sends them.
        private const val COMMAND_LEFT = 0xffe9
        private const val COMMAND_RIGHT = 0xffea
        private const val OPTION_LEFT = 0xffe7
        private const val OPTION_RIGHT = 0xffe8

        // Linux input event codes, which Android reports as a key's scan code, of physical keys and their USB HID usages,
        // as the kernel's HID driver maps them. The core turns a usage into the Mac's key code.
        private val USAGES = mapOf(
            30 to 0x04, 48 to 0x05, 46 to 0x06, 32 to 0x07, 18 to 0x08, 33 to 0x09, 34 to 0x0a, 35 to 0x0b, 23 to 0x0c,
            36 to 0x0d, 37 to 0x0e, 38 to 0x0f, 50 to 0x10, 49 to 0x11, 24 to 0x12, 25 to 0x13, 16 to 0x14, 19 to 0x15,
            31 to 0x16, 20 to 0x17, 22 to 0x18, 47 to 0x19, 17 to 0x1a, 45 to 0x1b, 21 to 0x1c, 44 to 0x1d,
            2 to 0x1e, 3 to 0x1f, 4 to 0x20, 5 to 0x21, 6 to 0x22, 7 to 0x23, 8 to 0x24, 9 to 0x25, 10 to 0x26, 11 to 0x27,
            28 to 0x28, 1 to 0x29, 14 to 0x2a, 15 to 0x2b, 57 to 0x2c, 12 to 0x2d, 13 to 0x2e, 26 to 0x2f, 27 to 0x30,
            43 to 0x31, 39 to 0x33, 40 to 0x34, 41 to 0x35, 51 to 0x36, 52 to 0x37, 53 to 0x38, 58 to 0x39,
            59 to 0x3a, 60 to 0x3b, 61 to 0x3c, 62 to 0x3d, 63 to 0x3e, 64 to 0x3f, 65 to 0x40, 66 to 0x41, 67 to 0x42,
            68 to 0x43, 87 to 0x44, 88 to 0x45,
            110 to 0x49, 102 to 0x4a, 104 to 0x4b, 111 to 0x4c, 107 to 0x4d, 109 to 0x4e, 106 to 0x4f, 105 to 0x50,
            108 to 0x51, 103 to 0x52,
            69 to 0x53, 98 to 0x54, 55 to 0x55, 74 to 0x56, 78 to 0x57, 96 to 0x58, 79 to 0x59, 80 to 0x5a, 81 to 0x5b,
            75 to 0x5c, 76 to 0x5d, 77 to 0x5e, 71 to 0x5f, 72 to 0x60, 73 to 0x61, 82 to 0x62, 83 to 0x63, 86 to 0x64,
            117 to 0x67,
            29 to 0xe0, 42 to 0xe1, 56 to 0xe2, 125 to 0xe3, 97 to 0xe4, 54 to 0xe5, 100 to 0xe6, 126 to 0xe7,
        )

        private val SPECIAL = mapOf(
            KeyEvent.KEYCODE_SHIFT_LEFT to 0xffe1,
            KeyEvent.KEYCODE_SHIFT_RIGHT to 0xffe2,
            KeyEvent.KEYCODE_CTRL_LEFT to 0xffe3,
            KeyEvent.KEYCODE_CTRL_RIGHT to 0xffe4,
            KeyEvent.KEYCODE_ESCAPE to 0xff1b,
            KeyEvent.KEYCODE_TAB to 0xff09,
            KeyEvent.KEYCODE_DEL to 0xff08,
            KeyEvent.KEYCODE_ENTER to 0xff0d,
            KeyEvent.KEYCODE_NUMPAD_ENTER to 0xff0d,
            KeyEvent.KEYCODE_FORWARD_DEL to 0xffff,
            KeyEvent.KEYCODE_INSERT to 0xff63,
            KeyEvent.KEYCODE_MOVE_HOME to 0xff50,
            KeyEvent.KEYCODE_MOVE_END to 0xff57,
            KeyEvent.KEYCODE_PAGE_UP to 0xff55,
            KeyEvent.KEYCODE_PAGE_DOWN to 0xff56,
            KeyEvent.KEYCODE_DPAD_LEFT to 0xff51,
            KeyEvent.KEYCODE_DPAD_UP to 0xff52,
            KeyEvent.KEYCODE_DPAD_RIGHT to 0xff53,
            KeyEvent.KEYCODE_DPAD_DOWN to 0xff54,
            KeyEvent.KEYCODE_SPACE to 0x20,
        ) + (0 until 12).associate { KeyEvent.KEYCODE_F1 + it to 0xffbe + it }
    }
}
