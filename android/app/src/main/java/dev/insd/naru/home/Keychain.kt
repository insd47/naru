package dev.insd.naru.home

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * 저장한 Mac의 비밀번호를 Android 키 저장소의 키로 암호화해 둡니다.
 *
 * - 키(AES-GCM)는 키 저장소 밖으로 나오지 않고, 앱 설정 파일에는 IV와 암호문만 남습니다.
 * - 키가 무효가 되어 풀 수 없는 비밀번호는 지우고 없는 것으로 봅니다. 그러면 다음 접속에서 다시 묻습니다.
 */
class Keychain(context: Context) {
    private val preferences = context.getSharedPreferences("passwords", Context.MODE_PRIVATE)

    fun find(hostId: String): String? {
        val sealed = preferences.getString(hostId, null)?.let { Base64.decode(it, Base64.NO_WRAP) } ?: return null

        return runCatching {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
            String(cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES), Charsets.UTF_8)
        }.onFailure {
            Log.w(TAG, "저장한 비밀번호를 풀지 못해 지웁니다", it)
            remove(hostId)
        }.getOrNull()
    }

    fun save(hostId: String, password: String) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val sealed = cipher.iv + cipher.doFinal(password.toByteArray(Charsets.UTF_8))
        preferences.edit().putString(hostId, Base64.encodeToString(sealed, Base64.NO_WRAP)).apply()
    }

    fun remove(hostId: String) {
        preferences.edit().remove(hostId).apply()
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }

        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .build()

        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER).apply { init(spec) }.generateKey()
    }

    companion object {
        private const val TAG = "Naru"
        private const val PROVIDER = "AndroidKeyStore"
        private const val ALIAS = "passwords"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val IV_BYTES = 12
        private const val TAG_BITS = 128
    }
}
