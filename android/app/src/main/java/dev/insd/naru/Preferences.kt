package dev.insd.naru

import android.content.Context
import androidx.compose.runtime.mutableStateOf
import kotlin.properties.ReadWriteProperty
import kotlin.reflect.KProperty

/**
 * Settings shared by every Mac, stored in the app's private preferences file.
 *
 * - Values are Compose state, so the settings screen redraws when they change. The session reads them on connect and the key mapping when the session screen opens, so a change takes effect from then on.
 */
class Preferences(context: Context) {
    private val store = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** Whether to send Alt as Command and Meta (the Windows key) as Option. When on, the keys sit where they do on a Mac keyboard. */
    var swapCommandOption by stored("swapCommandOption", default = true)

    /** Whether the Mac's input source follows the tablet's input language. Off by default, as in the Screen Sharing app. */
    var syncKeyboardLanguage by stored("syncKeyboardLanguage", default = false)

    private fun stored(key: String, default: Boolean) = object : ReadWriteProperty<Any?, Boolean> {
        private val state = mutableStateOf(store.getBoolean(key, default))

        override fun getValue(thisRef: Any?, property: KProperty<*>): Boolean = state.value

        override fun setValue(thisRef: Any?, property: KProperty<*>, value: Boolean) {
            state.value = value
            store.edit().putBoolean(key, value).apply()
        }
    }
}
