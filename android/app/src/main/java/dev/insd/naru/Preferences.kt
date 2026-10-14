package dev.insd.naru

import android.content.Context
import androidx.compose.runtime.mutableStateOf

/**
 * Settings shared by every Mac, stored in the app's private preferences file.
 *
 * - Values are Compose state, so the settings screen redraws when they change. The session reads them on connect and the key mapping when the session screen opens, so a change takes effect from then on.
 */
class Preferences(context: Context) {
    private val store = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val swapState = mutableStateOf(store.getBoolean(SWAP_COMMAND_OPTION, true))
    private val syncState = mutableStateOf(store.getBoolean(SYNC_KEYBOARD_LANGUAGE, false))

    /** Whether to send Alt as Command and Meta (the Windows key) as Option. When on, the keys sit where they do on a Mac keyboard. */
    var swapCommandOption: Boolean
        get() = swapState.value
        set(value) {
            swapState.value = value
            store.edit().putBoolean(SWAP_COMMAND_OPTION, value).apply()
        }

    /** Whether the Mac's input source follows the tablet's input language. Off by default, as in the Screen Sharing app. */
    var syncKeyboardLanguage: Boolean
        get() = syncState.value
        set(value) {
            syncState.value = value
            store.edit().putBoolean(SYNC_KEYBOARD_LANGUAGE, value).apply()
        }

    companion object {
        private const val SWAP_COMMAND_OPTION = "swapCommandOption"
        private const val SYNC_KEYBOARD_LANGUAGE = "syncKeyboardLanguage"
    }
}
