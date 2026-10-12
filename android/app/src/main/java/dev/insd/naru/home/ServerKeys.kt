package dev.insd.naru.home

import android.content.Context
import java.io.File

/**
 * Public keys of connected Macs, saved on first connection and checked for changes on later ones.
 *
 * - A Mac's public key has no certificate chain, so the first key seen is trusted and the user is asked if it later changes.
 */
class ServerKeys(context: Context) {
    private val directory = File(context.filesDir, "server-keys")

    fun find(host: Host): ByteArray? = file(host).takeIf { it.exists() }?.readBytes()

    /** Saves only when no key is stored yet. */
    fun remember(host: Host, der: ByteArray) {
        val file = file(host)

        if (!file.exists()) {
            directory.mkdirs()
            file.writeBytes(der)
        }
    }

    fun forget(host: Host) {
        file(host).delete()
    }

    private fun file(host: Host) = File(directory, "${host.address.replace(Regex("[^A-Za-z0-9.:-]"), "_")}.der")
}
