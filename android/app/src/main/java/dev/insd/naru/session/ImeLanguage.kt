package dev.insd.naru.session

import android.content.Context
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.inputmethod.InputMethodManager
import dev.insd.naru.TAG
import dev.insd.naru.protocol.Language

/**
 * 태블릿 IME의 입력 언어를 세션에 알립니다. Mac의 입력 소스를 지정하고 글자를 자모로 바꾸는 일은 세션이 합니다.
 *
 * - 한/영 키는 One UI가 처리하므로 키가 눌린 뒤 [sync]로 다시 읽습니다. 키를 시스템이 먹는 경우에 대비해 선택한 IME 서브타입의 변경도 지켜봅니다.
 * - 같은 언어는 다시 알리지 않습니다. 메인 스레드에서만 부르십시오.
 */
class ImeLanguage(private val context: Context, private val connection: Connection) {
    private val handler = Handler(Looper.getMainLooper())
    private var reported: Language? = null

    private val observer = object : ContentObserver(handler) {
        override fun onChange(selfChange: Boolean) = sync(delayMs = 0)
    }

    init {
        context.contentResolver.registerContentObserver(Settings.Secure.getUriFor(SELECTED_SUBTYPE), false, observer)
    }

    /** 입력 언어를 다시 읽어 바뀌었으면 알립니다. 한/영 키가 눌린 뒤에는 IME가 언어를 바꿀 때까지 조금 기다립니다. */
    fun sync(delayMs: Long = SETTLE_MS) {
        handler.postDelayed({ report(read()) }, delayMs)
    }

    fun stop() {
        handler.removeCallbacksAndMessages(null)
        context.contentResolver.unregisterContentObserver(observer)
    }

    private fun report(language: Language) {
        if (language == reported) {
            return
        }

        reported = language
        Log.i(TAG, "태블릿 입력 언어: $language")
        connection.language(language)
    }

    private fun read(): Language {
        val subtype = context.getSystemService(InputMethodManager::class.java).currentInputMethodSubtype
        val tag = subtype?.languageTag?.takeIf { it.isNotEmpty() } ?: subtype?.let { @Suppress("DEPRECATION") it.locale } ?: ""
        return if (tag.startsWith("ko")) Language.KOREAN else Language.ENGLISH
    }

    companion object {
        private const val SETTLE_MS = 150L

        // Settings.Secure.SELECTED_INPUT_METHOD_SUBTYPE는 공개 API에 없으므로 이름을 직접 씁니다.
        private const val SELECTED_SUBTYPE = "selected_input_method_subtype"
    }
}
