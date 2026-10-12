package dev.insd.naru.session

import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent

/**
 * Converts Android key events to X11 keysyms.
 *
 * - Modifiers map by position. On a PC layout the Alt next to the space bar sits where Command is on a Mac, so Alt → Command and
 *   Meta → Option. When [swapCommandOption] is `false`, they map by name (Alt → Option, Meta → Command).
 * - The Mac treats Super_L as Command and Meta_L as Option. It also treats Alt_L as Command, so Alt_L is not used for Option.
 * - Character keys send the physical key's Latin character. If the Mac's input source is a layout that cannot type Latin, such as Dubeolsik, the session converts to that layout's characters.
 * - Without Control, Option, or Command held, sends the shifted character (`A`, `!`); with any held, sends the lowercase one (`c`).
 * - A release sends the same keysym as the press, so no key stays held on the Mac if Shift changed in between.
 * - Dead keys (`´` then `e`) send the character composed by the tablet layout (`é`). The Mac inserts characters missing from its layout as is.
 * - Caps Lock is not sent. Case is already in the character, and if Caps Lock switches the Mac to ABC, Korean comes out as separate jamo.
 */
class Keys(swapCommandOption: Boolean) {
    private val pressed = HashMap<Int, Int>()

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

    /** The (keysym, down) pairs to send, empty for an unknown key */
    fun translate(event: KeyEvent): List<Pair<Int, Boolean>> {
        val down = when (event.action) {
            KeyEvent.ACTION_DOWN -> true
            KeyEvent.ACTION_UP -> false
            else -> return emptyList()
        }

        if (!down) {
            return pressed.remove(event.keyCode)?.let { listOf(it to false) } ?: emptyList()
        }

        val keysym = keysym(event) ?: return emptyList()
        pressed[event.keyCode] = keysym
        return listOf(keysym to true)
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
    fun releaseAll(): List<Pair<Int, Boolean>> = pressed.values.map { it to false }.also { pressed.clear() }

    private fun keysym(event: KeyEvent): Int? {
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
        return if (combined in 0x20..0x7e || combined in 0xa0..0xff) combined else 0x0100_0000 + combined
    }

    private fun char(event: KeyEvent): Int {
        val commandLike = KeyEvent.META_CTRL_ON or KeyEvent.META_ALT_ON or KeyEvent.META_META_ON
        val meta = if (event.metaState and commandLike != 0) 0 else event.metaState and KeyEvent.META_SHIFT_MASK
        return event.keyCharacterMap.get(event.keyCode, meta)
    }

    companion object {
        // Super_L/R are confirmed as Command and Meta_L as Option. Meta_R mirrors Meta_L and is unverified.
        private const val COMMAND_LEFT = 0xffeb
        private const val COMMAND_RIGHT = 0xffec
        private const val OPTION_LEFT = 0xffe7
        private const val OPTION_RIGHT = 0xffe8

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
