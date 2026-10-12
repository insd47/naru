package dev.insd.naru.home

import android.content.Context
import androidx.compose.runtime.mutableStateListOf
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** A saved Mac. [Keychain] keeps the password separately. */
data class Host(
    val id: String = UUID.randomUUID().toString(),
    /** Name given by the user. If empty, it is filled with the computer name the Mac reports on connect. */
    val name: String = "",
    val address: String,
    val username: String = "",
    val port: Int = DEFAULT_PORT,
    /** Whether to connect with a virtual display sized to the tablet screen. When off, the Mac's real display is shown. */
    val virtualDisplay: Boolean = true,
    /** Time of the last successful connection (epoch ms) */
    val lastConnected: Long? = null,
) {
    /** Name to display. Falls back to the address when there is no name. */
    val title: String get() = name.ifEmpty { address }

    companion object {
        const val DEFAULT_PORT = 5900
    }
}

/**
 * The list of saved Macs, stored as JSON in the app's private preferences file.
 *
 * - [list] is Compose state, so the screen redraws when it changes. Modified only on the main thread.
 * - One instance per process ([dev.insd.naru.NaruApplication]), so the list screen immediately sees what the session service saves on a successful connection.
 */
class Hosts(context: Context) {
    private val preferences = context.getSharedPreferences("hosts", Context.MODE_PRIVATE)

    val list = mutableStateListOf<Host>().apply { addAll(load()) }

    /** The Mac saved under this ID, or `null` */
    fun find(id: String): Host? = list.find { it.id == id }

    /** Replaces the entry with the same ID, or appends it if there is none. */
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

            // Older versions saved the address as the name of a Mac added without one. Read it as no name.
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
