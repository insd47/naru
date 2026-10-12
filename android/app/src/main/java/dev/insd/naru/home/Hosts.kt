package dev.insd.naru.home

import android.content.Context
import androidx.compose.runtime.mutableStateListOf
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** 저장한 Mac 하나. 비밀번호는 [Keychain]이 따로 둡니다. */
data class Host(
    val id: String = UUID.randomUUID().toString(),
    /** 사용자가 붙인 이름. 비어 있으면 접속할 때 Mac이 알려 준 컴퓨터 이름으로 채웁니다. */
    val name: String = "",
    val address: String,
    val username: String = "",
    val port: Int = DEFAULT_PORT,
    /** 태블릿 화면 크기의 가상 디스플레이로 접속할지 여부. 끄면 Mac의 실제 화면을 받습니다. */
    val virtualDisplay: Boolean = true,
    /** 마지막으로 접속에 성공한 시각(epoch ms) */
    val lastConnected: Long? = null,
) {
    /** 화면에 보일 이름. 이름이 없으면 주소입니다. */
    val title: String get() = name.ifEmpty { address }

    companion object {
        const val DEFAULT_PORT = 5900
    }
}

/**
 * 저장한 Mac 목록입니다. 앱 전용 설정 파일에 JSON으로 둡니다.
 *
 * - [list]는 Compose 상태라 바뀌면 화면이 다시 그려집니다. 메인 스레드에서만 고칩니다.
 * - 프로세스에 하나만 둡니다([dev.insd.naru.NaruApplication]). 세션 서비스가 접속에 성공하며 저장한 것도 목록 화면이 곧바로 봅니다.
 */
class Hosts(context: Context) {
    private val preferences = context.getSharedPreferences("hosts", Context.MODE_PRIVATE)

    val list = mutableStateListOf<Host>().apply { addAll(load()) }

    /** 이 ID로 저장한 Mac. 없으면 `null`입니다. */
    fun find(id: String): Host? = list.find { it.id == id }

    /** 같은 ID가 있으면 바꾸고, 없으면 끝에 더합니다. */
    fun save(host: Host) {
        val index = list.indexOfFirst { it.id == host.id }
        if (index >= 0) list[index] = host else list += host
        store()
    }

    fun remove(host: Host) {
        list.removeAll { it.id == host.id }
        store()
    }

    private fun load(): List<Host> {
        val array = JSONArray(preferences.getString(KEY, "[]"))

        return (0 until array.length()).map { index ->
            val item = array.getJSONObject(index)
            val address = item.getString("address")

            // 예전에는 이름 없이 추가한 Mac의 이름을 주소로 저장했습니다. 이름 없음으로 읽습니다.
            Host(
                id = item.getString("id"),
                name = item.getString("name").takeIf { it != address } ?: "",
                address = address,
                username = item.getString("username"),
                port = item.optInt("port", Host.DEFAULT_PORT),
                virtualDisplay = item.optBoolean("virtualDisplay", true),
                lastConnected = item.optLong("lastConnected").takeIf { it > 0 },
            )
        }
    }

    private fun store() {
        val array = JSONArray()

        for (host in list) {
            array.put(
                JSONObject()
                    .put("id", host.id)
                    .put("name", host.name)
                    .put("address", host.address)
                    .put("username", host.username)
                    .put("port", host.port)
                    .put("virtualDisplay", host.virtualDisplay)
                    .putOpt("lastConnected", host.lastConnected),
            )
        }

        preferences.edit().putString(KEY, array.toString()).apply()
    }

    companion object {
        private const val KEY = "hosts"
    }
}
