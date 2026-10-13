package dev.insd.naru.session

import android.content.Context
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.inputmethod.InputMethodManager
import dev.insd.naru.TAG

/**
 * Tracks the tablet IME's input language as a BCP 47 tag, which decides what character keys type on the Mac.
 *
 * - One UI handles the Korean/English key, so [sync] rereads the language after it is pressed. The selected IME subtype is also observed in case the system consumes the key.
 * - Call only on the main thread.
 */
class ImeLanguage(private val context: Context, private val onChange: (String) -> Unit = {}) {
    private val handler = Handler(Looper.getMainLooper())

    /** The current input language, empty when the IME reports none */
    var current: String = ""
        private set

    private val observer = object : ContentObserver(handler) {
        override fun onChange(selfChange: Boolean) = sync(delayMs = 0)
    }

    init {
        context.contentResolver.registerContentObserver(Settings.Secure.getUriFor(SELECTED_SUBTYPE), false, observer)
    }

    /** Rereads the input language and reports it if it changed. After the Korean/English key, waits briefly for the IME to switch. */
    fun sync(delayMs: Long = SETTLE_MS) {
        handler.postDelayed({ report(read()) }, delayMs)
    }

    fun stop() {
        handler.removeCallbacksAndMessages(null)
        context.contentResolver.unregisterContentObserver(observer)
    }

    private fun report(language: String) {
        if (language != current) {
            current = language
            Log.i(TAG, "Tablet input language: $language")
            onChange(language)
        }
    }

    // An empty tag when the subtype reports no language. Latin characters are then sent as is, as for an unknown language.
    private fun read(): String {
        val subtype = context.getSystemService(InputMethodManager::class.java).currentInputMethodSubtype
        return subtype?.languageTag?.takeIf { it.isNotEmpty() } ?: subtype?.let { @Suppress("DEPRECATION") it.locale } ?: ""
    }

    companion object {
        private const val SETTLE_MS = 150L

        // Settings.Secure.SELECTED_INPUT_METHOD_SUBTYPE is not public API, so the name is spelled out.
        private const val SELECTED_SUBTYPE = "selected_input_method_subtype"
    }
}
