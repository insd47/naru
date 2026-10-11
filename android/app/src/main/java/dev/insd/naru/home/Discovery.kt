package dev.insd.naru.home

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.mutableStateListOf

/** 같은 네트워크에서 찾은 화면 공유 Mac. 주소는 고를 때 풉니다. */
data class Found(val name: String, val service: NsdServiceInfo)

/**
 * 같은 네트워크의 화면 공유 Mac을 Bonjour(`_rfb._tcp`)로 찾습니다.
 *
 * - [found]는 Compose 상태입니다. NSD 콜백은 다른 스레드에서 오므로 메인 스레드로 넘겨 고칩니다.
 * - 화면이 보이는 동안만 [start]와 [stop]으로 찾습니다.
 */
class Discovery(context: Context) {
    private val manager = context.getSystemService(NsdManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var listener: NsdManager.DiscoveryListener? = null

    val found = mutableStateListOf<Found>()

    fun start() {
        if (listener != null) {
            return
        }

        val next = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(service: NsdServiceInfo) {
                main.post {
                    if (found.none { it.name == service.serviceName }) found += Found(service.serviceName, service)
                }
            }

            override fun onServiceLost(service: NsdServiceInfo) {
                main.post { found.removeAll { it.name == service.serviceName } }
            }

            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "네트워크 찾기를 시작하지 못했습니다: $errorCode")
                listener = null
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }

        listener = next
        manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, next)
    }

    fun stop() {
        val current = listener ?: return
        listener = null
        runCatching { manager.stopServiceDiscovery(current) }
        found.clear()
    }

    /** 찾은 Mac의 IPv4 주소를 풉니다. 결과는 메인 스레드에서 [done]으로 옵니다. 실패하면 `null`입니다. */
    @Suppress("DEPRECATION")
    fun resolve(target: Found, done: (String?) -> Unit) {
        manager.resolveService(target.service, object : NsdManager.ResolveListener {
            override fun onServiceResolved(service: NsdServiceInfo) {
                val address = service.host?.hostAddress?.takeIf { '.' in it }
                main.post { done(address) }
            }

            override fun onResolveFailed(service: NsdServiceInfo, errorCode: Int) {
                Log.w(TAG, "주소를 풀지 못했습니다: $errorCode")
                main.post { done(null) }
            }
        })
    }

    companion object {
        private const val TAG = "Naru"
        private const val SERVICE_TYPE = "_rfb._tcp."
    }
}
