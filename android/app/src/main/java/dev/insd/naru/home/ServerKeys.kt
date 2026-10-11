package dev.insd.naru.home

import android.content.Context
import java.io.File

/**
 * 접속한 Mac의 공개키입니다. 처음 접속할 때 저장하고, 다음 접속부터 바뀌었는지 확인하는 데 씁니다.
 *
 * - Mac의 공개키에는 인증서 체인이 없으므로, 처음 본 키를 믿고 이후 바뀌면 사용자에게 묻습니다.
 */
class ServerKeys(context: Context) {
    private val directory = File(context.filesDir, "server-keys")

    fun find(host: Host): ByteArray? = file(host).takeIf { it.exists() }?.readBytes()

    /** 저장한 키가 없을 때만 저장합니다. */
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
