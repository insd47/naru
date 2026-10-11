package dev.insd.naru

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.insd.naru.protocol.Priority

/**
 * 모든 Mac에 함께 쓰는 설정입니다. 앱 전용 설정 파일에 둡니다.
 *
 * - 값은 Compose 상태라 바뀌면 설정 화면이 다시 그려집니다. 세션은 접속할 때 읽으므로 바꾼 값은 다음 접속부터 씁니다.
 */
class Preferences(context: Context) {
    private val store = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** 패킷을 잃었을 때 지연과 화질 중 무엇을 우선할지. Screen Sharing 앱처럼 지연을 먼저 둡니다. */
    var priority by mutableStateOf(store.getString(PRIORITY, null)?.let(::priorityOf) ?: Priority.LATENCY)
        private set

    fun select(next: Priority) {
        priority = next
        store.edit().putString(PRIORITY, keyOf(next)).apply()
    }

    // 저장하는 값은 코어 열거형의 이름이 바뀌어도 그대로 읽히도록 따로 둡니다.
    private fun keyOf(priority: Priority) = when (priority) {
        Priority.LATENCY -> "latency"
        Priority.QUALITY -> "quality"
    }

    private fun priorityOf(key: String) = Priority.entries.find { keyOf(it) == key }

    companion object {
        private const val PRIORITY = "priority"
    }
}
