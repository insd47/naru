package dev.insd.naru.session

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.util.Log

/**
 * Mac과 태블릿의 클립보드 텍스트를 맞춥니다.
 *
 * - Mac에서 바뀐 텍스트는 받는 대로 태블릿 클립보드에 넣습니다.
 * - Android는 포커스를 가진 앱만 클립보드를 읽게 하므로, 세션 화면이 포커스를 얻을 때 읽어 Mac에 보냅니다.
 * - 마지막으로 주고받은 텍스트를 기억해 같은 내용을 되돌려 보내지 않습니다. 내용은 로그에 남기지 않습니다.
 */
class Clipboard(private val context: Context, private val connection: Connection) {
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private var last: String? = null

    /** Mac의 클립보드가 바뀌었습니다. */
    fun onServer(text: String) {
        last = text
        manager.setPrimaryClip(ClipData.newPlainText(LABEL, text))
    }

    /** 태블릿 클립보드의 텍스트가 Mac과 다르면 보냅니다. */
    fun sync() {
        val clip = manager.primaryClip?.takeIf { it.itemCount > 0 } ?: run {
            Log.i(TAG, "클립보드를 읽지 못했습니다")
            return
        }
        val text = clip.getItemAt(0).coerceToText(context).toString()

        if (text.isEmpty() || text == last) {
            return
        }

        last = text
        Log.i(TAG, "클립보드 ${text.length}자를 Mac에 보냅니다")
        connection.clipboard(text)
    }

    companion object {
        private const val TAG = "Naru"
        private const val LABEL = "Mac"
    }
}
