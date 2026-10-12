package dev.insd.naru.session

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.util.Log
import dev.insd.naru.TAG

/**
 * Keeps clipboard text in sync between the Mac and the tablet.
 *
 * - Text changed on the Mac goes into the tablet clipboard as soon as it arrives.
 * - Android lets only the focused app read the clipboard, so it is read and sent to the Mac when the session screen gains focus.
 * - The last text exchanged is remembered so the same content is not echoed back. Content is never logged.
 * - Text already present at connect is not sent; only text copied after connecting is.
 */
class Clipboard(private val context: Context, private val connection: Connection) {
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private var last: String? = read()

    /** The Mac's clipboard changed. If it is our own text echoed back, the tablet clipboard is left alone. */
    fun onServer(text: String) {
        if (text == last) {
            return
        }

        last = text
        manager.setPrimaryClip(ClipData.newPlainText(LABEL, text))
    }

    /** Sends the tablet clipboard text if it differs from the Mac's. */
    fun sync() {
        val text = read() ?: run {
            Log.i(TAG, "Failed to read the clipboard")
            return
        }

        if (text.isEmpty() || text == last) {
            return
        }

        if (!connection.clipboard(text)) {
            Log.w(TAG, "Not sending clipboard of ${text.length} chars: too large")
            return
        }

        last = text
        Log.i(TAG, "Sent clipboard of ${text.length} chars to the Mac")
    }

    // Android shows the clipboard only to the focused app, so call this only while the screen has focus.
    private fun read(): String? = manager.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()

    companion object {
        private const val LABEL = "Mac"
    }
}
