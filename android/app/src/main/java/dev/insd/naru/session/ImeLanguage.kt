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
 * Reports the tablet IME's input language to the session as a BCP 47 tag. The session maps characters to that language's Mac layout.
 *
 * - One UI handles the Korean/English key, so [sync] rereads the language after it is pressed. The selected IME subtype is also observed in case the system consumes the key.
 * - The same language is not reported twice. Call only on the main thread.
 */
class ImeLanguage(private val context: Context, private val connection: Connection) {
    private val handler = Handler(Looper.getMainLooper())
    private var reported: String? = null

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
        if (language == reported) {
            return
        }

        reported = language
        Log.i(TAG, "Tablet input language: $language")
        connection.language(language)
    }

    // An empty tag when the subtype reports no language. The session then sends Latin characters as is, as for an unknown language.
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
