package dev.insd.naru.session

import android.view.KeyCharacterMap
import android.view.KeyEvent

/**
 * Android 키 이벤트를 X11 keysym으로 바꿉니다.
 *
 * - 수식 키는 자리를 기준으로 옮깁니다. PC 배열에서 스페이스 옆의 Alt가 Mac 배열의 Command 자리이므로 Alt → Command,
 *   Meta → Option입니다. [swapCommandOption]이 `false`면 이름대로(Alt → Option, Meta → Command) 옮깁니다.
 * - Mac은 Super_L을 Command로, Meta_L을 Option으로 받습니다. Alt_L도 Command로 받으므로 Option에는 쓰지 않습니다.
 * - 글자 키는 물리 키의 라틴 문자를 보냅니다. Mac의 입력 소스가 한국어면 세션이 2벌식 자모로 바꿉니다.
 * - Control, Option, Command가 눌려 있지 않으면 Shift를 적용한 문자(`A`, `!`)를, 눌려 있으면 소문자(`c`)를 보냅니다.
 * - 뗄 때는 누를 때 보낸 keysym을 그대로 보냅니다. 그 사이 Shift가 바뀌어도 Mac에 눌린 키가 남지 않게 합니다.
 */
class Keys(private val swapCommandOption: Boolean) {
    private val pressed = HashMap<Int, Int>()

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

    /** 보낼 (keysym, 누름) 목록. 모르는 키면 빈 목록입니다. */
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

    /** 눌린 채로 남은 키를 모두 뗍니다. 화면을 떠날 때 Mac에 키가 눌린 채 남지 않게 합니다. */
    fun releaseAll(): List<Pair<Int, Boolean>> = pressed.values.map { it to false }.also { pressed.clear() }

    private fun keysym(event: KeyEvent): Int? {
        modifiers[event.keyCode]?.let { return it }
        SPECIAL[event.keyCode]?.let { return it }

        val commandLike = KeyEvent.META_CTRL_ON or KeyEvent.META_ALT_ON or KeyEvent.META_META_ON
        val meta = if (event.metaState and commandLike != 0) 0 else event.metaState and KeyEvent.META_SHIFT_MASK
        val char = event.keyCharacterMap.get(event.keyCode, meta)

        if (char == 0 || char and KeyCharacterMap.COMBINING_ACCENT != 0) {
            return null
        }

        // 라틴-1은 문자 코드 그대로, 그 밖은 유니코드 keysym입니다.
        return if (char in 0x20..0x7e || char in 0xa0..0xff) char else 0x0100_0000 + char
    }

    companion object {
        // Super_L/R은 Command, Meta_L은 Option으로 확인했습니다. Meta_R은 Meta_L에 맞춘 것으로 아직 확인하지 않았습니다.
        private const val COMMAND_LEFT = 0xffeb
        private const val COMMAND_RIGHT = 0xffec
        private const val OPTION_LEFT = 0xffe7
        private const val OPTION_RIGHT = 0xffe8

        private val SPECIAL = mapOf(
            KeyEvent.KEYCODE_SHIFT_LEFT to 0xffe1,
            KeyEvent.KEYCODE_SHIFT_RIGHT to 0xffe2,
            KeyEvent.KEYCODE_CTRL_LEFT to 0xffe3,
            KeyEvent.KEYCODE_CTRL_RIGHT to 0xffe4,
            KeyEvent.KEYCODE_CAPS_LOCK to 0xffe5,
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
