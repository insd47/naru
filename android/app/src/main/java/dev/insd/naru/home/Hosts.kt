package dev.insd.naru.home

import android.content.Context
import androidx.compose.runtime.mutableStateListOf
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** 저장한 Mac 하나. 비밀번호는 [Keychain]이 따로 둡니다. */
data class Host(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val address: String,
    val username: String = "",
    val port: Int = DEFAULT_PORT,
    /** 태블릿 화면 크기의 가상 디스플레이로 접속할지 여부. 끄면 Mac의 실제 화면을 받습니다. */
    val virtualDisplay: Boolean = true,
    /** 마지막으로 접속에 성공한 시각(epoch ms) */
    val lastConnected: Long? = null,
) {
    companion object {
        const val DEFAULT_PORT = 5900
    }
}

/**
 * 저장한 Mac 목록입니다. 앱 전용 설정 파일에 JSON으로 둡니다.
 *
 * - [list]는 Compose 상태라 바뀌면 화면이 다시 그려집니다. 메인 스레드에서만 고칩니다.
 * - 세션 화면도 접속에 성공하면 저장하므로, 목록 화면은 다시 보일 때 [reload]로 파일을 다시 읽습니다.
 */
class Hosts(context: Context) {
    private val preferences = context.getSharedPreferences("hosts", Context.MODE_PRIVATE)

    val list = mutableStateListOf<Host>().apply { addAll(load()) }

    fun reload() {
        val loaded = load()

        if (loaded != list) {
            list.clear()
            list.addAll(loaded)
        }
    }

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
            Host(
                id = item.getString("id"),
                name = item.getString("name"),
                address = item.getString("address"),
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
