package dev.insd.naru.session

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.inputmethod.InputMethodManager

/**
 * 태블릿의 한/영 상태를 Mac의 입력 소스로 옮깁니다.
 *
 * - 한/영 키는 One UI가 처리하므로, 키가 눌린 뒤 현재 IME의 언어를 읽어 한국어면 2벌식, 아니면 ABC를 Mac에 지정합니다.
 * - 서버가 바뀐 입력 소스를 알려 오면 [onChanged]로 알립니다. 한국어일 때는 글자 키를 자모로 보내야 Mac이 조합합니다.
 * - 메인 스레드에서만 부르십시오.
 */
class InputSource(
    private val context: Context,
    private val connection: Connection,
    private val onChanged: (korean: Boolean) -> Unit,
) {
    private val handler = Handler(Looper.getMainLooper())

    private var target: String? = null
    private var current: String? = null

    /** 태블릿의 언어를 다시 읽어 Mac에 반영합니다. 한/영 키가 눌린 뒤나 세션 시작 때 부릅니다. */
    fun sync(delayMs: Long = SETTLE_MS) {
        handler.postDelayed({ apply(read()) }, delayMs)
    }

    /** 서버가 알려 온 입력 소스 */
    fun onServer(id: String) {
        current = id
        onChanged(id.contains("Korean"))

        if (id == target) {
            Log.i(TAG, "입력 소스 지정 확인: $id")
            target = null
        }
    }

    fun stop() = handler.removeCallbacksAndMessages(null)

    private fun read(): String {
        val manager = context.getSystemService(InputMethodManager::class.java)
        val language = manager.currentInputMethodSubtype?.languageTag?.takeIf { it.isNotEmpty() }
            ?: manager.currentInputMethodSubtype?.let { @Suppress("DEPRECATION") it.locale }
            ?: ""
        Log.i(TAG, "태블릿 입력 언어: ${language.ifEmpty { "알 수 없음" }}")
        return if (language.startsWith("ko")) KOREAN else ENGLISH
    }

    private fun apply(id: String) {
        if (id == current) {
            return
        }

        target = id
        Log.i(TAG, "입력 소스 지정: $id")
        connection.inputSource(id)

        handler.postDelayed({
            if (target == id) {
                Log.w(TAG, "입력 소스 지정이 받아들여지지 않았습니다: $id")
                target = null
            }
        }, CONFIRM_MS)
    }

    companion object {
        private const val TAG = "Naru"
        private const val KOREAN = "com.apple.inputmethod.Korean.2SetKorean"
        private const val ENGLISH = "com.apple.keylayout.ABC"
        private const val SETTLE_MS = 150L
        private const val CONFIRM_MS = 1_500L
    }
}
